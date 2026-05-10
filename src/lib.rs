mod copy_extract;
mod generator;
mod mapper;
mod ontology;
mod parser;
mod renderer;
mod schema;

pub use copy_extract::{ExtractCopyOptions, extract_copy_blocks, extract_copy_to_csv_files};
pub use generator::{GenerateOptions, generate_file};

#[cfg(test)]
mod tests {
    use crate::mapper::map_schema_to_ontology;
    use crate::parser::parse_postgres_schema;
    use crate::renderer::render_module;
    use crate::{ExtractCopyOptions, extract_copy_blocks, extract_copy_to_csv_files};
    use std::fs;

    #[test]
    fn generates_mts_for_simple_table() {
        let sql = r#"
            CREATE TABLE employee (
                uuid UUID PRIMARY KEY,
                employee_number VARCHAR(32) NOT NULL,
                full_name TEXT,
                created_at TIMESTAMP NOT NULL
            );
        "#;

        let schema = parse_postgres_schema(sql).expect("schema should parse");
        let module = map_schema_to_ontology(&schema).expect("schema should map");
        let rendered = render_module(&module);

        assert!(rendered.contains("import { defineObject } from \"@osdk/maker\";"));
        assert!(module.warnings.is_empty());
        assert!(rendered.contains("export const employee = defineObject({"));
        assert!(rendered.contains("primaryKeyPropertyApiName: \"uuid\""));
        assert!(rendered.contains("titlePropertyApiName: \"fullName\""));
        assert!(rendered.contains(
            "\"employeeNumber\": { type: \"string\", displayName: \"Employee Number\" }"
        ));
        assert!(
            rendered
                .contains("\"createdAt\": { type: \"timestamp\", displayName: \"Created At\" }")
        );
    }

    #[test]
    fn generates_links_for_foreign_keys() {
        let sql = r#"
            CREATE TABLE "software_part_numbers" (
                "id" serial NOT NULL,
                "name" varchar(255) NOT NULL UNIQUE,
                "make_id" integer NOT NULL,
                PRIMARY KEY ("id")
            );
            CREATE TABLE "make" (
                "id" serial NOT NULL,
                PRIMARY KEY ("id")
            );
            ALTER TABLE "software_part_numbers"
                ADD CONSTRAINT "software_part_numbers_fk5"
                FOREIGN KEY ("make_id") REFERENCES "make"("id");
        "#;

        let schema = parse_postgres_schema(sql).expect("schema should parse");
        let module = map_schema_to_ontology(&schema).expect("schema should map");
        let rendered = render_module(&module);

        assert!(rendered.contains("import { defineLink, defineObject } from \"@osdk/maker\";"));
        assert!(module.warnings.is_empty());
        assert!(rendered.contains("export const softwarePartNumbers = defineObject({"));
        assert!(rendered.contains("displayName: \"Software Part Number\""));
        assert!(rendered.contains("pluralDisplayName: \"Software Part Numbers\""));
        assert!(rendered.contains("\"id\": { type: \"integer\", displayName: \"Id\" }"));
        assert!(rendered.contains("export const makeToSoftwarePartNumbers = defineLink({"));
        assert!(rendered.contains("manyForeignKeyProperty: \"makeId\""));
        assert!(rendered.contains("object: make,"));
        assert!(rendered.contains("object: softwarePartNumbers,"));
    }

    #[test]
    fn exposes_warning_when_primary_key_is_inferred() {
        let sql = r#"
            CREATE TABLE netflix_shows (
                show_id TEXT NOT NULL,
                title TEXT
            );
        "#;

        let schema = parse_postgres_schema(sql).expect("schema should parse");
        let module = map_schema_to_ontology(&schema).expect("schema should map");

        assert_eq!(module.warnings.len(), 1);
        assert!(module.warnings[0].contains("netflix_shows"));
        assert!(module.warnings[0].contains("show_id"));
    }

    #[test]
    fn supports_composite_primary_keys_with_synthetic_property() {
        let sql = r#"
            CREATE TABLE "PlaylistTrack" (
                "PlaylistId" INTEGER NOT NULL,
                "TrackId" INTEGER NOT NULL
            );
            ALTER TABLE ONLY "PlaylistTrack"
                ADD CONSTRAINT "PK_PlaylistTrack" PRIMARY KEY ("PlaylistId", "TrackId");
        "#;

        let schema = parse_postgres_schema(sql).expect("schema should parse");
        let module = map_schema_to_ontology(&schema).expect("schema should map");
        let rendered = render_module(&module);

        assert_eq!(module.warnings.len(), 1);
        assert!(module.warnings[0].contains("composite primary key"));
        assert!(rendered.contains("primaryKeyPropertyApiName: \"playlistIdTrackIdKey\""));
        assert!(rendered.contains("\"playlistIdTrackIdKey\": { type: \"string\""));
    }

    #[test]
    fn extracts_copy_blocks_into_csv_files() {
        let sql = r#"COPY public.netflix_shows (show_id, title) FROM stdin;
s1	Dick Johnson Is Dead
s2	Blood & Water
\.
"#;

        let parsed = extract_copy_blocks(sql).expect("copy blocks should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].table_name, "netflix_shows");

        let base_dir =
            std::env::temp_dir().join(format!("ontolosql-copy-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base_dir);
        fs::create_dir_all(&base_dir).expect("temp dir should be created");

        let input_path = base_dir.join("input.sql");
        fs::write(&input_path, sql).expect("input sql should be written");

        let output_dir = base_dir.join("csv");
        let outputs = extract_copy_to_csv_files(ExtractCopyOptions {
            input: input_path,
            output_dir: output_dir.clone(),
        })
        .expect("csv extraction should succeed");

        assert_eq!(outputs.len(), 1);
        let csv = fs::read_to_string(output_dir.join("netflix_shows.csv"))
            .expect("csv file should be readable");
        assert!(csv.contains("show_id,title"));
        assert!(csv.contains("s1,Dick Johnson Is Dead"));

        let _ = fs::remove_dir_all(&base_dir);
    }
}
