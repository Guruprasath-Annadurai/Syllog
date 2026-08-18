# Historical prototype (not built)

This directory contains Syllog's original hand-written lexer/parser experiment.
The root manifest is a virtual Cargo workspace and does not declare this as a
package, so these files are not compiled, tested, or authoritative.

Do not add features here. The active parser is `crates/syllog-parser`, its Pest
grammar is authoritative, and compilation is orchestrated by
`crates/syllog-compiler`. Archiving or deleting this history requires explicit
approval because it affects repository provenance.
