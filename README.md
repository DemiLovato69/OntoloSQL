# ontolosql

`ontolosql` is a Rust CLI that generates TypeScript ontology definition files for `@osdk/maker` from PostgreSQL schema files.

Current scope is intentionally narrow: it reads PostgreSQL DDL, extracts tables, columns, primary keys, and foreign keys, then emits:

- `defineObject(...)` blocks for tables
- `defineLink(...)` blocks for one-to-many foreign key relationships
- CSV files from `COPY ... FROM stdin` blocks in dump files
## Purpose

Allows the generation of ontologies from existing database sources and/or generated SQL from existing tools like [DBSchema](https://dbschema.com/download.html) or [DBDesigner](https://dbdesigner.net). Currently only supports PostGreSQL DDL's.

## What It Generates

For each supported table, `ontolosql` maps:

- table name -> object `apiName`, `displayName`, `pluralDisplayName`
- primary key column -> `primaryKeyPropertyApiName`
- a best-effort human-readable column -> `titlePropertyApiName`
- columns -> `properties`
- foreign keys -> `defineLink(...)`

If a table does not declare a primary key, `ontolosql` will try to infer one from a not-null identifier column such as `id` or `show_id` and print a warning to `stderr`.

Generated output targets `@osdk/maker` imports like:

```ts
import { defineLink, defineObject } from "@osdk/maker";
```

For pg_dump-style `COPY` blocks it can also write one CSV per table for use as Foundry backing datasets.

## Usage

Build and run:

```bash
cargo run -- generate --input schema.sql --output index.mts
cargo run -- extract-copy --input dump.sql --output-dir datasets
```

CLI help:

```bash
cargo run -- --help
cargo run -- generate --help
```

## Supported Input

The generator currently handles these schema elements:

- `CREATE TABLE`
- inline column definitions
- single-column primary keys
- foreign keys declared inline on columns
- foreign keys declared with table constraints
- `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY ...`

It also tolerates unrelated PostgreSQL statements in the same file and skips them when they are not relevant to ontology generation, including:

- `CREATE EXTENSION`
- `CREATE TYPE ... AS ENUM`
- `CREATE INDEX`
- `CREATE FUNCTION`
- `CREATE TRIGGER`

This is important for real schema dumps that contain operational SQL in addition to table definitions.

For dump extraction, `ontolosql` also supports:

- `COPY ... FROM stdin;`
- tab-delimited PostgreSQL text rows
- `\N` null markers
- common PostgreSQL backslash escapes in field values

## Type Mapping

Current PostgreSQL to OSDK type mapping:

- `uuid`, `text`, `varchar`, `char`, `citext` -> `string`
- `smallint`, `integer`, `int`, `serial`, `smallserial` -> `integer`
- `bigint`, `bigserial` -> `long`
- `numeric`, `decimal`, `real`, `double precision` -> `double`
- `boolean` -> `boolean`
- `date` -> `date`
- `timestamp`, `timestamptz` -> `timestamp`

Unknown or unsupported SQL types currently fall back to `string`.

## Naming Behavior

The generator normalizes SQL names into OSDK-friendly TypeScript identifiers:

- `software_part_numbers` -> `softwarePartNumbers`
- `stock_size_bytes` -> `stockSizeBytes`
- quoted SQL identifiers are unwrapped before mapping

For display names it also applies a light singular/plural heuristic:

- `software_part_numbers` -> `Software Part Number` / `Software Part Numbers`
- `companies` -> `Company` / `Companies`
- `map_data` stays `Map Data`

## Current Limitations

This is still an MVP. Known limits:

- composite primary keys are not supported
- composite foreign keys are not supported
- inferred primary keys are best-effort heuristics, not guaranteed relational keys
- enum definitions are ignored and enum-backed columns are emitted as `string`
- no action types, interfaces, or shared property types are generated
- no special handling yet for multiple foreign keys that would collide on link naming
- no schema-aware namespace or repository integration yet
- `COPY` extraction currently writes plain CSV files only; it does not generate Foundry dataset metadata or schemas yet

## Development

Run tests:

```bash
cargo test
```

Format:

```bash
cargo fmt
```

## Code Layout

- [src/main.rs](src/main.rs): CLI entrypoint
- [src/generator.rs](src/generator.rs): file orchestration
- [src/copy_extract.rs](src/copy_extract.rs): pg_dump `COPY` to CSV extraction
- [src/parser.rs](src/parser.rs): PostgreSQL parsing
- [src/schema.rs](src/schema.rs): internal schema IR
- [src/mapper.rs](src/mapper.rs): SQL IR -> ontology IR
- [src/ontology.rs](src/ontology.rs): internal ontology IR
- [src/renderer.rs](src/renderer.rs): `.mts` rendering

## Next Steps

The next meaningful improvements are:

1. Generate enum-aware property definitions instead of flattening enum types to `string`.
2. Make link naming collision-safe when several foreign keys point between the same tables.
3. Add configuration for title-property selection and naming conventions.
4. Generate Foundry-ready dataset metadata alongside extracted CSVs.
