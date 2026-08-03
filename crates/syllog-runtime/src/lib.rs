//! WebAssembly isolation boundary used by future `evo` execution.

use anyhow::Context;
use std::collections::HashSet;
use wasmtime::{Config, Engine, Linker, Module, ResourceLimiter, Store, Trap};

const WASM_PAGE_BYTES: u64 = 64 * 1024;

/// Per-invocation resource policy for untrusted WebAssembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicy {
    fuel: u64,
    max_memory_bytes: usize,
    capabilities: HashSet<HostCapability>,
}

impl SandboxPolicy {
    /// Creates a policy with finite instruction fuel and linear-memory bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when either limit is zero.
    pub fn new(fuel: u64, max_memory_bytes: usize) -> Result<Self, PolicyError> {
        if fuel == 0 {
            return Err(PolicyError::ZeroFuel);
        }
        if max_memory_bytes == 0 {
            return Err(PolicyError::ZeroMemory);
        }
        Ok(Self {
            fuel,
            max_memory_bytes,
            capabilities: HashSet::new(),
        })
    }

    /// Grants one explicit host capability to the module.
    #[must_use]
    pub fn allow(mut self, capability: HostCapability) -> Self {
        self.capabilities.insert(capability);
        self
    }
}

/// Host functions that a sandbox policy can grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostCapability {
    /// Allows the deterministic `syllog::log_i32(i32)` diagnostic sink.
    LogI32,
}

impl HostCapability {
    fn import(self) -> (&'static str, &'static str) {
        match self {
            Self::LogI32 => ("syllog", "log_i32"),
        }
    }
}

/// Invalid Wasm sandbox policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// No instructions could execute.
    #[error("sandbox fuel must be greater than zero")]
    ZeroFuel,
    /// No linear memory could be allocated.
    #[error("sandbox memory limit must be greater than zero")]
    ZeroMemory,
}

/// Failure while preparing or executing sandboxed WebAssembly.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Module bytes failed validation or compilation.
    #[error("invalid sandboxed WebAssembly module")]
    Compilation(#[source] anyhow::Error),
    /// The store could not accept the requested resource policy.
    #[error("failed to configure the WebAssembly store")]
    Configuration(#[source] anyhow::Error),
    /// Imports could not be linked or the module could not be instantiated.
    #[error("failed to instantiate sandboxed WebAssembly")]
    Instantiation(#[source] anyhow::Error),
    /// The requested export was absent or had a different signature.
    #[error("missing or invalid i32 export '{export}'")]
    InvalidExport {
        /// Requested export name.
        export: String,
        /// Wasmtime lookup error.
        #[source]
        source: anyhow::Error,
    },
    /// The module consumed all policy fuel.
    #[error("WebAssembly execution exhausted its fuel allowance")]
    FuelExhausted,
    /// The module's declared or requested linear memory exceeded policy.
    #[error("WebAssembly linear memory exceeds its policy allowance")]
    MemoryLimitExceeded,
    /// The module requested a host import not granted by policy.
    #[error("host capability '{module}::{name}' is not allowed")]
    CapabilityDenied {
        /// Import module namespace.
        module: String,
        /// Import field name.
        name: String,
    },
    /// The export trapped for a reason other than fuel exhaustion.
    #[error("WebAssembly export '{export}' trapped")]
    Execution {
        /// Requested export name.
        export: String,
        /// Wasmtime execution error.
        #[source]
        source: anyhow::Error,
    },
}

/// A deterministic Wasmtime compilation sandbox.
#[derive(Clone)]
pub struct Sandbox {
    engine: Engine,
}

#[derive(Debug)]
struct HostState {
    limiter: MemoryLimiter,
}

#[derive(Debug)]
struct MemoryLimiter {
    max_bytes: usize,
    exceeded: bool,
}

impl ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        if desired > self.max_bytes {
            self.exceeded = true;
            anyhow::bail!("linear-memory policy limit exceeded");
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }
}

impl Sandbox {
    /// Creates an engine with fuel accounting enabled.
    ///
    /// # Errors
    ///
    /// Returns an error when the host cannot initialize Wasmtime.
    pub fn new() -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.cranelift_nan_canonicalization(true);
        let engine = Engine::new(&config).context("failed to initialize Wasmtime")?;
        Ok(Self { engine })
    }

    /// Compiles and validates a WebAssembly module without instantiating it.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed or unsupported WebAssembly bytes.
    pub fn compile(&self, bytes: &[u8]) -> anyhow::Result<Module> {
        Module::new(&self.engine, bytes).context("invalid sandboxed WebAssembly module")
    }

    /// Executes a no-argument WebAssembly export returning `i32`.
    ///
    /// # Errors
    ///
    /// Returns an error when compilation, instantiation, export lookup, or
    /// execution fails.
    pub fn execute_i32(
        &self,
        bytes: &[u8],
        export: &str,
        policy: &SandboxPolicy,
    ) -> Result<i32, SandboxError> {
        let module = self.compile(bytes).map_err(SandboxError::Compilation)?;
        for import in module.imports() {
            let granted = policy
                .capabilities
                .iter()
                .any(|capability| capability.import() == (import.module(), import.name()));
            if !granted {
                return Err(SandboxError::CapabilityDenied {
                    module: import.module().into(),
                    name: import.name().into(),
                });
            }
        }
        let initial_memory_bytes = module
            .resources_required()
            .max_initial_memory_size
            .unwrap_or(0)
            .saturating_mul(WASM_PAGE_BYTES);
        if initial_memory_bytes > policy.max_memory_bytes as u64 {
            return Err(SandboxError::MemoryLimitExceeded);
        }

        let mut store = Store::new(
            &self.engine,
            HostState {
                limiter: MemoryLimiter {
                    max_bytes: policy.max_memory_bytes,
                    exceeded: false,
                },
            },
        );
        store.limiter(|state| &mut state.limiter);
        store
            .set_fuel(policy.fuel)
            .map_err(SandboxError::Configuration)?;
        let mut linker = Linker::new(&self.engine);
        if policy.capabilities.contains(&HostCapability::LogI32) {
            linker
                .func_wrap("syllog", "log_i32", |value: i32| {
                    tracing::debug!(value, "sandbox log_i32");
                })
                .map_err(SandboxError::Instantiation)?;
        }
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(SandboxError::Instantiation)?;
        let function = instance
            .get_typed_func::<(), i32>(&mut store, export)
            .map_err(|source| SandboxError::InvalidExport {
                export: export.into(),
                source,
            })?;
        match function.call(&mut store, ()) {
            Ok(value) => Ok(value),
            Err(_source) if store.data().limiter.exceeded => Err(SandboxError::MemoryLimitExceeded),
            Err(source) if source.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) => {
                Err(SandboxError::FuelExhausted)
            }
            Err(source) => Err(SandboxError::Execution {
                export: export.into(),
                source,
            }),
        }
    }
}
