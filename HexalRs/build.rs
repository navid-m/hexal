use std::fs;
use std::path::Path;
use std::process::Command;

fn commit_hash(manifest: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(["rev-parse", "--short", "HEAD"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if hash.is_empty() {
                "unknown".into()
            } else {
                hash
            }
        }
        _ => "unknown".into(),
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let grammar_dir = Path::new(&manifest).join("data").join("grammars");
    println!("cargo:rerun-if-changed={}", grammar_dir.display());
    let plugin_dir = Path::new(&manifest).join("data").join("plugins");
    println!("cargo:rerun-if-changed={}", plugin_dir.display());
    copy_plugins(&plugin_dir);

    println!(
        "cargo:rustc-env=HEXAL_COMMIT_HASH={}",
        commit_hash(&manifest)
    );
    if let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(&manifest)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
    {
        if output.status.success() {
            let directory = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !directory.is_empty() {
                println!("cargo:rerun-if-changed={directory}");
            }
        }
    }

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

/// Copies the editable plugin defaults next to the built binary, mirroring the
/// C# project's `data/plugins` publish layout. Plugin contents are never
/// embedded in the executable.
fn copy_plugins(source: &Path) {
    if !source.is_dir() {
        return;
    }
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let Some(profile_dir) = Path::new(&out_dir).ancestors().nth(3) else {
        return;
    };
    let destination = profile_dir.join("data").join("plugins");
    let _ = fs::remove_dir_all(&destination);
    if let Err(error) = copy_tree(source, &destination) {
        println!("cargo:warning=cannot copy plugin data: {error}");
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
