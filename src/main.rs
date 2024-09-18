mod app;
mod args;
mod py;

const PROMPT1: &str = "pyapp > ";
const PROMPT2: &str = " .... > ";
const PROMPT1_OK: &str = "\x1b[1;94mpyapp\x1b[1;32m > \x1b[m";
const PROMPT1_ERR: &str = "\x1b[1;94mpyapp\x1b[1;91m > \x1b[m";
const PROMPT2_OK: &str = "\x1b[1;94m ....\x1b[1;32m > \x1b[m";
const TERMINATE_N: u8 = 2;

fn main() -> app::ExitCode {
    // // cli/run -m ipykernel_launcher --f=~/.local/share/jupyter/runtime/kernel-v2-28141kSC9njpUbH1b.json
    app::run(args::Args::parse())
}
