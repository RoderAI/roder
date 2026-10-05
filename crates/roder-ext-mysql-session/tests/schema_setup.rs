//! Requires an isolated MySQL test server whose user can create databases/users.
use roder_ext_mysql_session::{MysqlSessionConfig, MysqlSessionStore, schema};
use sqlx_core::sql_str::AssertSqlSafe;
use sqlx_core::{connection::Connection, query::query, query_scalar::query_scalar};
use sqlx_mysql::{MySql, MySqlPoolOptions};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires isolated MySQL with database/user creation privileges"]
async fn runtime_connections_require_setup_and_do_not_wait_for_migration_locks() {
    let url = std::env::var("RODER_MYSQL_TEST_URL").expect("isolated MySQL URL");
    let admin = MySqlPoolOptions::new().connect(&url).await.unwrap();
    // DDL identifiers below contain only fixed prefixes and UUID hex digits.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let database = format!("roder_schema_{suffix}");
    let user = format!("runtime_{}", &suffix[..16]);
    query::<MySql>(AssertSqlSafe(format!("CREATE DATABASE `{database}`")))
        .execute(&admin)
        .await
        .unwrap();
    let (base, _) = url.rsplit_once('/').unwrap();
    let fixture_url = format!("{base}/{database}");
    let config = MysqlSessionConfig::new(&fixture_url, "schema-test").unwrap();
    let pool = MySqlPoolOptions::new().connect(&fixture_url).await.unwrap();

    let error = match MysqlSessionStore::connect(&config).await {
        Ok(_) => panic!("an uninitialized schema must be rejected"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("roder-mysql-migrate"));
    let tables: i64 = query_scalar::<MySql, i64>(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE()",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tables, 0, "runtime connect must not create schema");

    schema::migrate(&pool).await.unwrap();
    query::<MySql>("DELETE FROM roder_session_migrations WHERE version = ?")
        .bind(schema::MIGRATION_VERSION)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        MysqlSessionStore::connect(&config).await.is_err(),
        "outdated schema must be rejected"
    );
    schema::migrate(&pool).await.unwrap();

    // A runtime account has no DDL privileges. Holding the historical migration
    // lock separately also detects GET_LOCK even though MySQL permits it without
    // schema-change privileges.
    query::<MySql>(AssertSqlSafe(format!(
        "CREATE USER '{user}'@'%' IDENTIFIED BY 'isolated-test-only'"
    )))
    .execute(&admin)
    .await
    .unwrap();
    query::<MySql>(AssertSqlSafe(format!(
        "GRANT SELECT, INSERT, UPDATE, DELETE ON `{database}`.* TO '{user}'@'%'"
    )))
    .execute(&admin)
    .await
    .unwrap();
    let (_, host) = base
        .rsplit_once('@')
        .expect("test URL includes credentials");
    let runtime = MysqlSessionConfig::new(
        format!("mysql://{user}:isolated-test-only@{host}/{database}"),
        "schema-test",
    )
    .unwrap();
    let mut lock = pool.acquire().await.unwrap().detach();
    let acquired: Option<i64> = query_scalar::<MySql, Option<i64>>(
        "SELECT GET_LOCK(CONCAT('roder-v2-', MD5(DATABASE())), 0)",
    )
    .fetch_one(&mut lock)
    .await
    .unwrap();
    assert_eq!(acquired, Some(1));
    let first = tokio::time::timeout(Duration::from_secs(3), MysqlSessionStore::connect(&runtime))
        .await
        .expect("runtime connection waited for migration lock")
        .unwrap();
    let second = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || MysqlSessionStore::connect_blocking(&runtime)),
    )
    .await
    .expect("blocking runtime connection waited for migration lock")
    .unwrap()
    .unwrap();
    assert!(first.runtime_owner().await.unwrap().is_none());
    assert!(second.runtime_owner().await.unwrap().is_none());
    drop((first, second));
    lock.close().await.unwrap();
    pool.close().await;
    query::<MySql>(AssertSqlSafe(format!("DROP USER '{user}'@'%'")))
        .execute(&admin)
        .await
        .unwrap();
    query::<MySql>(AssertSqlSafe(format!("DROP DATABASE `{database}`")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
