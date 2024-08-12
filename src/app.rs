use crate::{py, ExecMode, PROMPT1, PROMPT2, TERMINATE_N};
use pyo3::{
    types::{PyAnyMethods, PyModule},
    PyErr, Python,
};
use rustyline::{
    config::Configurer, error::ReadlineError, highlight::Highlighter,
    hint::HistoryHinter, history::DefaultHistory, Cmd, Completer, Editor, EventHandler,
    Helper, Hinter, KeyCode, KeyEvent, Modifiers, Validator,
};
use std::borrow::Cow::{self, Borrowed, Owned};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use thiserror::Error;

pub(super) fn run(mode: ExecMode) -> ExitCode {
    use py::foo;
    pyo3::append_to_inittab!(foo);
    pyo3::prepare_freethreaded_python();
    ExitCode {
        inner: match mode {
            ExecMode::InteractiveShell => run_shell(),
            ExecMode::ExecFile { quiet: true, path, args } => quiet_exec_file(path, args),
            ExecMode::ExecFile { quiet: false, path, args } => exec_file(path, args),
        },
    }
}

pub(crate) struct ExitCode {
    inner: Result<(), ExecErr>,
}

#[derive(Error, Debug)]
enum ExecErr {
    #[error("data store disconnected")]
    PyResult(#[from] PyErr),
    #[error("data store disconnected")]
    Readline(#[from] ReadlineError),
    #[error("data store disconnected")]
    IO(#[from] std::io::Error),
}

impl std::process::Termination for ExitCode {
    fn report(self) -> std::process::ExitCode {
        match self.inner {
            Ok(()) => 0.into(),
            Err(ExecErr::PyResult(e)) => {
                println!("{}", e);
                1.into()
            }
            Err(ExecErr::Readline(e)) => {
                println!("{}", e);
                1.into()
            }
            Err(ExecErr::IO(e)) => {
                println!("{}", e);
                1.into()
            }
        }
    }
}

#[derive(Completer, Helper, Hinter, Validator)]
struct MyHelper(#[rustyline(Hinter)] HistoryHinter);

impl Highlighter for MyHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Owned(format!("\x1b[1;32m{prompt}\x1b[m"))
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned(format!("\x1b[1m{hint}\x1b[m"))
    }
}

fn run_shell() -> Result<(), ExecErr> {
    let mut rl = Editor::<MyHelper, DefaultHistory>::new()?;
    rl.set_helper(Some(MyHelper(HistoryHinter::new())));
    rl.set_auto_add_history(true);
    rl.bind_sequence(
        KeyEvent(KeyCode::Char('s'), Modifiers::CTRL),
        EventHandler::Simple(Cmd::Newline),
    );
    let mut code = String::new();
    let mut prompt = PROMPT1;
    let mut terminate_count: u8 = 0;
    Python::with_gil(|py| {
        let compile_command =
            PyModule::import_bound(py, "codeop")?.getattr("compile_command")?;
        loop {
            let readline = rl.readline(prompt);
            match readline {
                Ok(line) => {
                    terminate_count = 0;
                    if !code.is_empty() {
                        code += "\n";
                    }
                    code += &line;
                    if let Ok(true) = py::is_incomplete_code(&compile_command, &code) {
                        prompt = PROMPT2;
                    } else {
                        prompt = PROMPT1;
                        if let Err(e) = py.run_bound(&code, None, None) {
                            println!("{}", e);
                        }
                        code.clear();
                    }
                }
                Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                    if terminate_count >= TERMINATE_N {
                        println!("\nExiting...");
                        return Ok(());
                    }
                    println!(
                        "Need {} interrupt to exit..",
                        TERMINATE_N - terminate_count
                    );
                    terminate_count += 1;
                }
                Err(err) => {
                    println!("Error: {:?}", err);
                    return Err(ExecErr::Readline(err));
                }
            }
        }
    })
}

fn exec_file(path: PathBuf, args: Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        py::import_args(py, args)?;
        let compile_command =
            PyModule::import_bound(py, "codeop")?.getattr("compile_command")?;
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut ping_pong_line_buffer = [String::new(), String::new()];
        let mut ping_pong_idx = true;
        let mut read_res = reader.read_line(&mut ping_pong_line_buffer[0]);
        let mut prompt = PROMPT1;
        let mut code = String::new();
        loop {
            let (this_idx, next_idx) = if read_res? == 0 {
                if !code.is_empty() {
                    if let Err(e) = py.run_bound(&code, None, None) {
                        println!("{}", e);
                    }
                }
                return Ok(());
            } else {
                if ping_pong_idx {
                    ping_pong_idx = false;
                    (0, 1)
                } else {
                    ping_pong_idx = true;
                    (1, 0)
                }
            };
            read_res = reader.read_line(&mut ping_pong_line_buffer[next_idx]);
            if let Ok(0) = read_res {
                ping_pong_line_buffer[this_idx] += "\n";
            }
            print!("{prompt}{}", ping_pong_line_buffer[this_idx]);
            code += &ping_pong_line_buffer[this_idx];
            ping_pong_line_buffer[this_idx].clear();
            let code_check_str: &str = match read_res {
                Ok(0) => &code,
                Err(_) => &code,
                _ => {
                    if ping_pong_line_buffer[next_idx].starts_with(&[' ', '\t', '\n']) {
                        &code[..code.len() - 1]
                    } else {
                        &code
                    }
                }
            };
            if let Ok(true) = py::is_incomplete_code(&compile_command, code_check_str) {
                prompt = PROMPT2;
            } else {
                prompt = PROMPT1;
                if let Err(e) = py.run_bound(&code, None, None) {
                    println!("{}", e);
                }
                code.clear();
            }
        }
    })
}

fn quiet_exec_file(path: PathBuf, args: Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        py::import_args(py, args)?;
        let mut file = File::open(path)?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        py.run_bound(buf.as_str(), None, None)?;
        Ok(())
    })
}

mod test {
    use super::*;
    #[test]
    fn test_exec_file() {
        use py::foo;
        pyo3::append_to_inittab!(foo);
        pyo3::prepare_freethreaded_python();
        exec_file(PathBuf::from("tests/test1.py"), vec![]);
    }
}
