fn is_help(argument: &str) -> bool {
    matches!(
        argument.to_ascii_lowercase().as_str(),
        "help" | "--help" | "-help" | "/help" | "--h" | "-h" | "/h" | "-?" | "/?"
    )
}

pub fn show(args: &[String]) -> bool {
    let Some((first, rest)) = args.split_first() else {
        return false;
    };
    let (flag, inline_topic) = first
        .split_once('=')
        .map_or((first.as_str(), None), |(flag, topic)| (flag, Some(topic)));
    let topic = if is_help(flag) {
        inline_topic.or_else(|| rest.first().map(String::as_str))
    } else if matches!(flag, "inspect" | "export-skill")
        && rest.first().is_some_and(|argument| is_help(argument))
    {
        Some(flag)
    } else {
        return false;
    };

    #[cfg(windows)]
    attach_to_console();

    match topic.map(str::to_ascii_lowercase).as_deref() {
        Some("agents" | "agents.md" | "skill") => crate::skill::print_guide(),
        Some("inspect") => println!(
            "inspect FILE|- [JOB.json]\n\
             Inspect G-code from a file or standard input (-). The optional JSON job\n\
             sets machine inputs, checks, execution limits and evidence to export.\n\
             Prints JSON. Exit 0 = passed, 2 = failed or incomplete, 1 = input error.\n\
             Run --help agents for the job schema and examples."
        ),
        Some("export-skill") => println!(
            "export-skill DIRECTORY\n\
             Extract the embedded guide, the skill pointer and Python helpers.\n\
             Use --help agents to read the guide directly from this executable."
        ),
        _ => println!(
            "CNC Viewer {}\n\
             Open G-code in the desktop viewer or inspect it from an agent workflow.\n\n\
             [FILE]                    Open the desktop viewer\n\
             --no-restore              Open a fresh window and keep the saved tabs\n\
             inspect FILE|- [JOB.json] Inspect G-code and optionally export evidence\n\
             --help agents            Print the complete CNC Viewer agent guide\n\
             export-skill DIRECTORY   Extract the guide and optional Python helpers",
            env!("CARGO_PKG_VERSION")
        ),
    }
    true
}

#[cfg(windows)]
fn attach_to_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
        fn AttachConsole(process: u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // The desktop subsystem may have no console. Preserve a caller's pipe or file.
    unsafe {
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        if stdout.is_null() || stdout as isize == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
