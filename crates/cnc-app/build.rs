use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn markdown_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            markdown_files(&path, files)?;
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn embed_skill() -> io::Result<()> {
    let root = Path::new("../../skills/cnc-viewer");
    println!("cargo:rerun-if-changed={}", root.display());
    let root = env::current_dir()?.join(root);
    let mut files = Vec::new();
    markdown_files(&root, &mut files)?;
    files.sort_unstable();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output"));
    let mut manifest = fs::File::create(output.join("skill_markdown.rs"))?;
    writeln!(manifest, "const MARKDOWN: &[(&str, &str)] = &[")?;
    for path in files {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        writeln!(manifest, "    ({name:?}, include_str!({path:?})),")?;
    }
    writeln!(manifest, "];")
}

fn main() {
    embed_skill().expect("embed CNC Viewer Markdown");
    println!("cargo:rerun-if-changed=assets/cnc-viewer.rc");
    println!("cargo:rerun-if-changed=assets/cnc-viewer.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/cnc-viewer.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile CNC Viewer application icon");
    }
}
