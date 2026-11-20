use std::{fs, path::Path};

include!(concat!(env!("OUT_DIR"), "/skill_markdown.rs"));

pub fn print_guide() {
    for (name, text) in MARKDOWN {
        print!("<!-- file: {name} -->\n\n{text}\n");
    }
}

pub fn export(destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let helpers = [
        (
            "scripts/inspect.py",
            include_str!("../../../skills/cnc-viewer/scripts/inspect.py"),
        ),
        (
            "agents/openai.yaml",
            include_str!("../../../skills/cnc-viewer/agents/openai.yaml"),
        ),
    ];
    let files = MARKDOWN.iter().copied().chain(helpers);
    let executable = std::env::current_exe()?;
    for name in files
        .clone()
        .map(|(name, _)| name)
        .chain(["executable.txt"])
    {
        if destination.join(name).exists() {
            return Err(
                format!("Export would overwrite {name}; choose an empty destination").into(),
            );
        }
    }
    fs::create_dir_all(destination)?;
    for (name, text) in files {
        let path = destination.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }
    fs::write(
        destination.join("executable.txt"),
        executable.to_string_lossy().as_bytes(),
    )?;
    Ok(())
}
