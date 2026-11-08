//! Writes the generated TypeScript and Swift protocol types (checked in).
//! `cargo run -p openpedal-protocol --bin gen-protocol`

use std::path::Path;

use openpedal_protocol::codegen;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let outputs = [
        ("apps/desktop/ui/src/generated/protocol.ts", codegen::typescript()),
        ("apps/ios/Sources/OpenPedalApp/Generated/Protocol.swift", codegen::swift()),
    ];
    for (path, text) in outputs {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(&file, text)?;
        println!("wrote {path}");
    }
    Ok(())
}
