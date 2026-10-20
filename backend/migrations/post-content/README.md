# Post-content migrations

Startup runs in this order (#207):

1. `backend/migrations/*.sql`, by sqlx, recorded in `_sqlx_migrations`
2. the content sync: `backend/content/*.toml` into the static tables (`resource_types`, `item_types`, `station_types`, `recipes`, ...)
3. `backend/migrations/post-content/*.sql`, by `migrations::run_post_content`, recorded in `_post_content_migrations`

**What goes here:** a migration that writes world rows naming content, such as a resource node of a resource type, a station of a station type, or an item of an item type. In the first set, that fails its foreign key on any database that hasn't seen the content yet. Here, the content exists.

**What stays in the first set:** schema changes, including the columns the content sync writes, and data that names no content.

## Rules

- **Name files the same way:** `<YYYYMMDD>_<NNN>_<what>.sql`. The version is the date prefix. This set has its own table, so its versions can't collide with the first set's. They run in version order, each once.
- **Rows a post-content migration wrote are only changed here.** A fresh database runs every first-set migration before any post-content one. A first-set migration that moves or deletes these rows would then run before the rows exist, and an existing database and a new one would differ. Put the move here, after the migration that wrote the rows.
- **Write against the schema as it stands when you write it.** A later first-set migration that drops or renames a column a post-content migration uses breaks a fresh database. `migrations::tests::the_post_content_migrations_apply_after_the_content_sync` runs this set on a fresh database, so it fails first.
- **Never edit an applied migration.** A change to its line endings is forgiven, but any other edit stops startup. Write a new one.
- **One transaction.** The whole run is one transaction, so a failure leaves the database as it was. `-- no-transaction` isn't supported.
- **A missing type should fail loudly.** Insert without joining `resource_types` and the rest. Startup stops on a row whose type the content doesn't have, rather than dropping the row without a word.

New default-world nodes, from `print_new_nodes` in `models/resource.rs`, go here as a plain `INSERT INTO resource_nodes ... ON CONFLICT (id) DO NOTHING`.
