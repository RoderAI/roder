use roder_ext_gbrain::schema::{migrate, register_sqlite_vec, sqlite_vec_available};
use rusqlite::Connection;

#[test]
fn dream_schema_migration_is_idempotent_and_creates_graph_tables() {
    let conn = Connection::open_in_memory().unwrap();

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    for table in [
        "gbrain_import_runs",
        "gbrain_import_manifest",
        "gbrain_dream_runs",
        "gbrain_statements",
        "gbrain_entities",
        "gbrain_predicates",
        "gbrain_ontology_nodes",
        "gbrain_ontology_edges",
        "gbrain_temporal_events",
        "gbrain_evidence_cards",
        "gbrain_dream_links",
        "gbrain_relevance",
        "gbrain_nodes",
        "gbrain_edges",
        "gbrain_hyperedges",
        "gbrain_node_aliases",
        "gbrain_id_remaps",
        "gbrain_communities",
        "gbrain_community_members",
        "gbrain_query_feedback",
    ] {
        assert!(
            table_exists(&conn, table),
            "expected dream schema table {table}"
        );
    }

    if sqlite_vec_available(&conn) {
        for table in [
            "gbrain_vec_raw_facts",
            "gbrain_vec_temporal_events",
            "gbrain_vec_evidence_cards",
            "gbrain_vec_ontology_nodes",
        ] {
            assert!(table_exists(&conn, table), "expected vec0 table {table}");
        }
    }
}

#[test]
fn graph_edge_endpoint_foreign_keys_are_enforced() {
    let conn = Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();

    conn.execute(
        "INSERT INTO gbrain_nodes(id, label, node_kind, confidence, created_at)
         VALUES ('node:a', 'A', 'person', 'EXTRACTED', '2026-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    let missing_target = conn.execute(
        "INSERT INTO gbrain_edges(id, source_node_id, target_node_id, relation, confidence, created_at)
         VALUES ('edge:missing', 'node:a', 'node:missing', 'owns', 'EXTRACTED', '2026-01-01T00:00:00Z')",
        [],
    );
    assert!(missing_target.is_err());

    let self_edge = conn.execute(
        "INSERT INTO gbrain_edges(id, source_node_id, target_node_id, relation, confidence, created_at)
         VALUES ('edge:self', 'node:a', 'node:a', 'owns', 'EXTRACTED', '2026-01-01T00:00:00Z')",
        [],
    );
    assert!(self_edge.is_err());
}

#[test]
fn sqlite_vec_registration_reports_availability_and_vec0_works_when_available() {
    let first = Connection::open_in_memory().unwrap();
    let status = register_sqlite_vec(&first);
    assert!(status.registered);

    let conn = Connection::open_in_memory().unwrap();
    if !sqlite_vec_available(&conn) {
        return;
    }

    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.gbrain_vec_smoke USING vec0(
           id TEXT PRIMARY KEY,
           embedding FLOAT[2]
         );
         INSERT INTO temp.gbrain_vec_smoke(id, embedding)
         VALUES ('a', '[0.0, 1.0]'), ('b', '[1.0, 0.0]');",
    )
    .unwrap();

    let id: String = conn
        .query_row(
            "SELECT id FROM temp.gbrain_vec_smoke
             WHERE embedding MATCH '[0.0, 1.0]'
             ORDER BY distance
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(id, "a");
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master WHERE type IN ('table', 'virtual table') AND name = ?1
         )",
        [table],
        |row| row.get::<_, bool>(0),
    )
    .unwrap()
}
