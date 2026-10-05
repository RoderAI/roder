use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("RODER_MYSQL_SESSION_URL")
        .context("Set RODER_MYSQL_SESSION_URL for the database to migrate")?;
    let pool = sqlx_mysql::MySqlPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .context("Connect to the MySQL session database")?;
    let result = roder_ext_mysql_session::schema::migrate(&pool).await;
    pool.close().await;
    result?;
    println!(
        "MySQL session schema is ready (version {})",
        roder_ext_mysql_session::schema::MIGRATION_VERSION
    );
    Ok(())
}
