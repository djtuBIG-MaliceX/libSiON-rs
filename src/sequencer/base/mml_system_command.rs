//! Port of `libSiON-cpp/src/sequencer/base/mml_system_command.h` — a system
//! command that cannot be parsed by the system.

#[derive(Clone, Debug, Default)]
pub struct MMLSystemCommand {
    /// For the given MML string `"#ABC5{def}ghi;"`...
    /// Command name; always starts with "#", e.g. `command = "#ABC"`.
    pub command: String,
    /// Number after command, e.g. `number = 5`.
    pub number: i32,
    /// String inside `{..}`, e.g. `content = "def"`.
    pub content: String,
    /// String at the end of the command, e.g. `postfix = "ghi"`.
    pub postfix: String,
}
