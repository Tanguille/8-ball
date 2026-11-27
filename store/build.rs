// sqlx::migrate!() embeds the migrations at build time, so a changed or new file must rebuild the crate.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
