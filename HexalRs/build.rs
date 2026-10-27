use std::fs;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let grammar_dir = Path::new(&manifest).join("data").join("grammars");
    println!("cargo:rerun-if-changed={}", grammar_dir.display());

    let mut files: Vec<_> = fs::read_dir(&grammar_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().is_some_and(|extension| {
                        extension.to_string_lossy().eq_ignore_ascii_case("hexdef")
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    let mut output = String::from("pub static EMBEDDED_GRAMMARS: &[(&str, &str)] = &[\n");
    for path in files {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        output.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            name,
            path.to_string_lossy()
        ));
    }
    output.push_str("];\n");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(Path::new(&out_dir).join("grammars.rs"), output).expect("write grammars.rs");
}
