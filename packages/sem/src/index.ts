// Types for the three Apache Ossie documents: a semantic model
// (`OssieDocument`), an ontology (`OntologyDocument`) and a mapping of one onto
// the other (`MappingDocument`). Everything under ./generated is generated
// from the Rust types in crates/ossie-model, so it accepts exactly what the
// Rust parser accepts.
export * from "./generated/index.js";
