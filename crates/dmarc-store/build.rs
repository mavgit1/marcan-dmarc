// `sqlx::migrate!` embeds migrations/ at compile time; rebuild when it changes,
// or a new migration file is silently left out of the binary.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
