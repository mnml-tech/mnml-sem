# @mnml-tech/sem

TypeScript types for the three [Apache Ossie](https://github.com/apache/ossie) documents:

- `OssieDocument`, a semantic model;
- `OntologyDocument`, an ontology;
- `MappingDocument`, a mapping of one onto the other.

The types are generated from the Rust types in [`ossie-model`](../../crates/ossie-model), so they accept exactly what the Rust parser accepts. They also include the spec version constants.

```ts
import type { OssieDocument } from "@mnml-tech/sem";

const model = {
  version: "0.2.0.dev0",
  name: "shop",
  datasets: [{ name: "orders", source: "warehouse.orders" }],
} satisfies OssieDocument;
```

Everything under `src/generated` and `test/fixtures` is generated, so do not edit it. To regenerate it from the repository root:

```sh
UPDATE_BINDINGS=1 cargo test -p ossie-model --features ts --test typescript
```

Then, in this directory, run `npm run typecheck`. It checks every fixture in `crates/ossie-model/tests/fixtures` against the types, and checks that the documents in `test/rejects.ts` are rejected.
