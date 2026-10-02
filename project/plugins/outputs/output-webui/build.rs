fn main() {
    println!("cargo:rerun-if-changed=assets");
    assert!(
        std::path::Path::new("assets/index.html").exists(),
        "Embedded frontend missing. Run npm ci && npm run build in frontend/."
    );
}
