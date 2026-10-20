// Documents the types must reject. Each is wrong in one way, marked with
// `@ts-expect-error`, so the typecheck fails if the types ever accept it: the
// fixtures passing proves nothing if every document passes.
import type { MappingDocument, OntologyDocument, OssieDocument } from "../src/index.js";

export const unknownKey = {
  version: "0.2.0.dev0",
  name: "m",
  datasets: [],
  // @ts-expect-error The schema has no `dimensions`.
  dimensions: [],
} satisfies OssieDocument;

export const unknownDialect = {
  version: "0.2.0.dev0",
  name: "m",
  datasets: [
    {
      name: "d",
      source: "s.d",
      fields: [
        {
          name: "f",
          // @ts-expect-error POSTGRES is not a dialect.
          expression: { dialects: [{ dialect: "POSTGRES", expression: "f" }] },
        },
      ],
    },
  ],
} satisfies OssieDocument;

export const unknownConceptType = {
  version: "0.2.0.dev0",
  name: "o",
  // @ts-expect-error A concept is an EntityType or a ValueType.
  ontology: [{ concept: "Customer", type: "Thing" }],
} satisfies OntologyDocument;

export const missingReference = {
  version: "0.2.0.dev0",
  name: "m",
  ontology_ref: { name: "o" },
  concept_mappings: [],
  // @ts-expect-error A mapping names the semantic model it reads.
} satisfies MappingDocument;
