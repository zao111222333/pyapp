use clap::{builder::styling::Styles, ArgGroup, Parser};
use std::path::PathBuf;

mod app;
mod py;

const PROMPT1: &str = "pyapp > ";
const PROMPT2: &str = " .... > ";
const PROMPT1_OK: &str = "\x1b[1;94mpyapp\x1b[1;32m > \x1b[m";
const PROMPT1_ERR: &str = "\x1b[1;94mpyapp\x1b[1;91m > \x1b[m";
const PROMPT2_OK: &str = "\x1b[1;94m ....\x1b[1;32m > \x1b[m";
const PROMPT2_OK_NEWLINE: &str = "\n\x1b[1;94m ....\x1b[1;32m > \x1b[m";
const PROMPT2_ERR: &str = "\x1b[1;94m ....\x1b[1;91m > \x1b[m";
const TERMINATE_N: u8 = 2;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "An example application",
    override_usage = "myapp [option] ... [-c cmd | -m mod | file | -] [arg] ...",
    long_about = "An example application ...",
    styles = CLAP_STYLING,
    group(ArgGroup::new("execution")
        // .required(true)
        // .requires_all(&["command", "module", "file"])
        .args(&["command", "module", "file"])),
)]
struct Args {
    /// run library module as a script (terminates option list)
    #[arg(short = 'm', value_name = "mod")]
    module: Option<String>,
    /// program passed in as string (terminates option list)
    #[arg(short = 'c', value_name = "cmd")]
    command: Option<String>,
    /// Python script to execute
    #[arg(value_name = "file")]
    file: Option<String>,
    // /// program read from stdin (default; interactive mode if a tty)
    // #[arg(value_name = "-", hide = true)]
    // shell: (),
    /// execute in quiet mode (effect in file mode)
    #[arg(short = 'q', long = "quiet", default_value_t = false)]
    quiet: bool,
    /// isolate Python from the user's environment (implies -E and -s)
    #[arg(short = 'I')]
    isolate: bool,
    /// don't add user site directory to sys.path; also PYTHONNOUSERSITE
    #[arg(short = 's')]
    ignore_site: bool,
    /// ignore PYTHON* environment variables (such as PYTHONPATH)
    #[arg(short = 'E')]
    ignore_env: bool,
    /// execute file with arguments. If not specify, start interactive shell mode
    #[arg(value_name = "arg", trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

pub const CLAP_STYLING: Styles = Styles::styled();

#[derive(Debug)]
enum ExecMode {
    /// Start in interactive shell mode
    InteractiveShell,
    /// Execute a file with arguments
    ExecFile { quiet: bool, file: PathBuf, args: Vec<String> },
    /// Execute a module with arguments
    Module { module: String, args: Vec<String> },
    /// Execute a command
    Command(String),
}

impl From<Args> for ExecMode {
    #[inline]
    fn from(mut value: Args) -> Self {
        if value.isolate {
            value.ignore_site = true;
            value.ignore_env = true;
            value.quiet = true;
        }

        if let Some(module) = value.module {
            ExecMode::Module {
                module,
                args: {
                    let mut args = vec![String::new()];
                    args.extend(value.args);
                    args
                },
            }
        } else if let Some(command) = value.command {
            ExecMode::Command(command)
        } else if let Some(file) = value.file {
            ExecMode::ExecFile {
                quiet: value.quiet,
                file: PathBuf::from(&file),
                args: {
                    let mut args = vec![file];
                    args.extend(value.args);
                    args
                },
            }
        } else {
            ExecMode::InteractiveShell
        }
    }
}

fn main() -> app::ExitCode {
    let args = Args::parse().into();
    app::run(args)
}
