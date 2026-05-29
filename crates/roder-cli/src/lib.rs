#[cfg(not(test))]
include!("main.rs");

#[cfg(not(test))]
pub fn run() -> anyhow::Result<()> {
    main()
}

#[cfg(test)]
pub fn run() -> anyhow::Result<()> {
    Ok(())
}
