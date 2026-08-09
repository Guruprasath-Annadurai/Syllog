//! CLI compilation commands sharing the production compiler pipeline.

pub mod add;
pub mod build;
pub mod dev;
pub mod fetch;
pub mod inspect;
pub mod new;
pub mod publish;
pub mod run;
pub mod test;
pub mod vendor;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use semver::VersionReq;
use syllog_compiler::{
    PackageSource, compile, compile_package, lower_to_hir, lower_to_mir, render_human,
};
use syllog_package::{ContentAddressedCache, LockedPackage, Resolution, read_lockfile};
use syllog_project::TargetKind;
use syllog_registry_client::PackageArchive;

pub(crate) fn compile_to_mir(path: &Path) -> anyhow::Result<Option<syllog_ir::MirProgram>> {
    let paths = project_source_paths(path)?;
    let has_dependencies = syllog_project::discover(path)
        .is_ok_and(|project| !project.manifest.dependencies.is_empty());
    if paths.len() > 1 || has_dependencies {
        return compile_package_to_mir(path, &paths);
    }
    let path = paths.first().map_or(path, PathBuf::as_path);
    let source =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let compilation = compile(path.display().to_string(), &source);
    if !compilation.success() {
        eprint!("{}", render_human(&source, &compilation.diagnostics));
        return Ok(None);
    }
    let ast = compilation
        .ast
        .as_ref()
        .expect("successful compilation has AST");
    let symbols = compilation
        .symbols
        .as_ref()
        .expect("successful compilation has symbols");
    let hir = lower_to_hir(ast, symbols)
        .map_err(|diagnostics| anyhow::anyhow!("HIR lowering failed: {diagnostics:#?}"))?;
    let mir = lower_to_mir(&hir)
        .map_err(|diagnostics| anyhow::anyhow!("MIR lowering failed: {diagnostics:#?}"))?;
    Ok(Some(mir))
}

fn compile_package_to_mir(
    requested: &Path,
    paths: &[PathBuf],
) -> anyhow::Result<Option<syllog_ir::MirProgram>> {
    let project =
        syllog_project::discover(requested).context("could not discover Syllog project")?;
    let mut source_text = BTreeMap::new();
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let source = fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let file = path
            .strip_prefix(&project.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        source_text.insert(file.clone(), source.clone());
        sources.push(PackageSource { file, source });
    }
    let dependencies = load_locked_dependency_sources(&project)?;
    source_text.extend(
        dependencies
            .iter()
            .map(|source| (source.file.clone(), source.source.clone())),
    );
    sources.extend(dependencies);
    let compilation = compile_package(sources);
    if !compilation.success() {
        for diagnostic in &compilation.diagnostics {
            let source = source_text.get(&diagnostic.file).map_or("", String::as_str);
            eprint!("{}", render_human(source, std::slice::from_ref(diagnostic)));
        }
        return Ok(None);
    }
    let hir = compilation.hir.expect("successful package has linked HIR");
    let mir = lower_to_mir(&hir)
        .map_err(|diagnostics| anyhow::anyhow!("MIR lowering failed: {diagnostics:#?}"))?;
    Ok(Some(mir))
}

fn load_locked_dependency_sources(
    project: &syllog_project::Project,
) -> anyhow::Result<Vec<PackageSource>> {
    if project.manifest.dependencies.is_empty() {
        return Ok(Vec::new());
    }
    let resolution = read_lockfile(&project.root.join("Syllog.lock"))
        .context("dependencies require a valid Syllog.lock")?;
    if resolution.format != 1 {
        anyhow::bail!("unsupported Syllog.lock format {}", resolution.format);
    }
    validate_lock_graph(project, &resolution)?;
    let vendor = project.root.join("vendor");
    let cache = ContentAddressedCache::new(project.root.join(".syllog/cache"));
    let mut sources = Vec::new();
    for package in &resolution.packages {
        let bytes = if vendor.is_dir() {
            let path = vendor_package_root(&vendor, package).join(".syllog-package");
            fs::read(&path).with_context(|| {
                format!(
                    "vendored package {} {} is incomplete: missing {}",
                    package.name,
                    package.version,
                    path.display()
                )
            })?
        } else {
            cache.load(&package.checksum).with_context(|| {
                format!(
                    "locked package {} {} is unavailable in the offline cache",
                    package.name, package.version
                )
            })?
        };
        let archive = PackageArchive::from_bytes(&bytes)
            .with_context(|| format!("invalid archive for package {}", package.name))?;
        verify_locked_archive(package, &archive)?;
        for file in archive.files {
            if Path::new(&file.path)
                .extension()
                .is_some_and(|extension| extension == "syl")
            {
                let source = String::from_utf8(file.content).with_context(|| {
                    format!("package {} source {} is not UTF-8", package.name, file.path)
                })?;
                sources.push(PackageSource {
                    file: format!(
                        "dependencies/{}-{}/{}",
                        package.name, package.version, file.path
                    ),
                    source,
                });
            }
        }
    }
    Ok(sources)
}

pub(crate) fn verify_locked_archive(
    package: &LockedPackage,
    archive: &PackageArchive,
) -> anyhow::Result<()> {
    if archive.name != package.name || archive.version != package.version {
        anyhow::bail!(
            "archive identity {} {} does not match lockfile package {} {}",
            archive.name,
            archive.version,
            package.name,
            package.version
        );
    }
    let actual = archive.checksum()?;
    if actual != package.checksum {
        anyhow::bail!(
            "archive checksum mismatch for {} {}: expected {}, computed {}",
            package.name,
            package.version,
            package.checksum,
            actual
        );
    }
    let archive_dependencies = archive.dependencies.keys().collect::<Vec<_>>();
    let locked_dependencies = package.dependencies.keys().collect::<Vec<_>>();
    if archive_dependencies != locked_dependencies {
        anyhow::bail!(
            "archive dependency metadata for {} {} does not match Syllog.lock",
            package.name,
            package.version
        );
    }
    for (name, requirement) in &archive.dependencies {
        let requirement = VersionReq::parse(requirement).with_context(|| {
            format!(
                "archive {} {} has invalid requirement for {name}",
                package.name, package.version
            )
        })?;
        if !requirement.matches(&package.dependencies[name]) {
            anyhow::bail!(
                "locked dependency {name} {} does not satisfy archive requirement {requirement}",
                package.dependencies[name]
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_lock_graph(
    project: &syllog_project::Project,
    resolution: &Resolution,
) -> anyhow::Result<()> {
    let mut packages = BTreeMap::new();
    for package in &resolution.packages {
        if packages.insert(&package.name, package).is_some() {
            anyhow::bail!("Syllog.lock contains duplicate package '{}'", package.name);
        }
    }
    for (name, dependency) in &project.manifest.dependencies {
        let package = packages
            .get(name)
            .with_context(|| format!("Syllog.lock does not contain direct dependency '{name}'"))?;
        let requirement = VersionReq::parse(&dependency.requirement).with_context(|| {
            format!("manifest has invalid version requirement for dependency '{name}'")
        })?;
        if !requirement.matches(&package.version) {
            anyhow::bail!(
                "locked package {name} {} does not satisfy manifest requirement {requirement}",
                package.version
            );
        }
    }
    for package in &resolution.packages {
        for (dependency, expected_version) in &package.dependencies {
            let resolved = packages.get(dependency).with_context(|| {
                format!(
                    "Syllog.lock omits dependency '{dependency}' required by {} {}",
                    package.name, package.version
                )
            })?;
            if resolved.version != *expected_version {
                anyhow::bail!(
                    "Syllog.lock selects {} for {dependency}, but {} {} requires {expected_version}",
                    resolved.version,
                    package.name,
                    package.version
                );
            }
        }
    }
    let mut reachable = BTreeSet::new();
    let mut pending = project
        .manifest
        .dependencies
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    while let Some(name) = pending.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        if let Some(package) = packages.get(&name) {
            pending.extend(package.dependencies.keys().cloned());
        }
    }
    if let Some(package) = resolution
        .packages
        .iter()
        .find(|package| !reachable.contains(&package.name))
    {
        anyhow::bail!(
            "Syllog.lock contains unreachable package '{}'; regenerate the lockfile",
            package.name
        );
    }
    Ok(())
}

fn vendor_package_root(vendor: &Path, package: &LockedPackage) -> PathBuf {
    vendor.join(format!("{}-{}", package.name, package.version))
}

fn project_source_paths(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if path.is_file() {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("could not resolve {}", path.display()))?;
        let Ok(project) = syllog_project::discover(path) else {
            return Ok(vec![canonical]);
        };
        if let Some(target) = project
            .manifest
            .targets
            .iter()
            .find(|target| target.path == canonical)
        {
            return collect_syl_sources(target.path.parent().unwrap_or(&project.root));
        }
        return Ok(vec![canonical]);
    }
    let project = syllog_project::discover(path).context("could not discover Syllog project")?;
    let target = project
        .manifest
        .targets
        .iter()
        .find(|target| target.kind == TargetKind::Bin)
        .or_else(|| project.manifest.targets.first())
        .context("project has no build targets")?;
    collect_syl_sources(target.path.parent().unwrap_or(&project.root))
}

fn collect_syl_sources(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_owned()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("could not read source directory {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "syl")
            {
                sources.push(entry.path());
            }
        }
    }
    sources.sort();
    if sources.is_empty() {
        anyhow::bail!("no .syl sources found below {}", root.display());
    }
    Ok(sources)
}
