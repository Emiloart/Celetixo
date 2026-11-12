use celetixo_engine::credentials::CredentialPolicy;
use celetixo_engine::ids::Digest;
fn main() {
    let root = std::env::args().nth(1).unwrap();
    let policy = CredentialPolicy::default();
    for path in std::env::args().skip(2) {
        assert!(!celetixo_engine::credentials::credential_path(&path), "private path must not be read in this public-tree audit");
        let bytes=std::fs::read(std::path::Path::new(&root).join(&path)).unwrap();
        if policy.admit_input(&path,&bytes).is_err() {
            println!("{}={}",path,Digest::of("celetixo/source-content/v1",&bytes));
        }
    }
}
