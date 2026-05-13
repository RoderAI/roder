package memory

import (
	"context"
	"database/sql"
)

const schemaVersion = 1

func migrate(ctx context.Context, db *sql.DB) error {
	var version int
	if err := db.QueryRowContext(ctx, `PRAGMA user_version`).Scan(&version); err != nil {
		return err
	}
	if version >= schemaVersion {
		return nil
	}
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	statements := []string{
		`CREATE TABLE IF NOT EXISTS memory_workspaces (
			id TEXT PRIMARY KEY,
			root TEXT NOT NULL UNIQUE,
			created_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL
		)`,
		`CREATE TABLE IF NOT EXISTS memories (
			id TEXT PRIMARY KEY,
			workspace_id TEXT NOT NULL,
			content TEXT NOT NULL,
			content_hash TEXT NOT NULL,
			source TEXT NOT NULL,
			metadata_json TEXT NOT NULL DEFAULT '{}',
			created_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			deleted_at INTEGER,
			FOREIGN KEY(workspace_id) REFERENCES memory_workspaces(id)
		)`,
		`CREATE TABLE IF NOT EXISTS memory_embeddings (
			memory_id TEXT NOT NULL,
			model TEXT NOT NULL,
			dimensions INTEGER NOT NULL,
			embedding BLOB NOT NULL,
			created_at INTEGER NOT NULL,
			PRIMARY KEY(memory_id, model),
			FOREIGN KEY(memory_id) REFERENCES memories(id)
		)`,
		`CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_workspace_hash
			ON memories(workspace_id, content_hash)
			WHERE deleted_at IS NULL`,
		`CREATE INDEX IF NOT EXISTS idx_memories_workspace_updated
			ON memories(workspace_id, updated_at DESC)
			WHERE deleted_at IS NULL`,
		`PRAGMA user_version = 1`,
	}
	for _, statement := range statements {
		if _, err := tx.ExecContext(ctx, statement); err != nil {
			return err
		}
	}
	return tx.Commit()
}
