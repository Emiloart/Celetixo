//! Offline-capable checked-in binding generator; not part of normal builds.
use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("repository root argument required")?);
    let protoc = PathBuf::from(args.next().ok_or("verified protoc argument required")?);
    let output = PathBuf::from(args.next().ok_or("output directory argument required")?);
    if args.next().is_some() {
        return Err("unexpected generator argument".into());
    }
    std::fs::create_dir_all(output.join("rust"))?;
    let mut config = prost_build::Config::new();
    config
        .protoc_executable(protoc)
        .out_dir(output.join("rust"))
        .file_descriptor_set_path(output.join("descriptor.bin"))
        .skip_source_info()
        .compile_protos(
            &[
                root.join("proto/celetixo/orchestration/v1/task_intent.proto"),
                root.join("proto/celetixo/platform/v1/task_intent_envelope.proto"),
            ],
            &[root.join("proto")],
        )?;
    Ok(())
}
