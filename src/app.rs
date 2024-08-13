use crate::{
    py, ExecMode, PROMPT1, PROMPT1_ERR, PROMPT1_OK, PROMPT2, PROMPT2_ERR, PROMPT2_OK,
    PROMPT2_OK_NEWLINE, TERMINATE_N,
};
use pyo3::{
    types::{PyAnyMethods, PyModule},
    PyErr, Python,
};
use rustyline::{
    config::Configurer, error::ReadlineError, highlight::Highlighter,
    hint::HistoryHinter, history::DefaultHistory, Cmd, Completer, Editor, EventHandler,
    Helper, Hinter, KeyCode, KeyEvent, Modifiers, Movement, Validator,
};
use std::{
    borrow::Cow::{self, Borrowed, Owned},
    fs::File,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
};
use thiserror::Error;

pub(super) fn run(mode: ExecMode) -> ExitCode {
    use py::foo;
    pyo3::append_to_inittab!(foo);
    pyo3::prepare_freethreaded_python();
    match mode {
        ExecMode::InteractiveShell => ExitCode { inner: run_shell(), path: None },
        ExecMode::ExecFile { quiet: true, path, args } => ExitCode {
            inner: quiet_exec_file(&path, args),
            path: Some(path),
        },
        ExecMode::ExecFile { quiet: false, path, args } => {
            ExitCode { inner: exec_file(&path, args), path: Some(path) }
        }
    }
}

pub(crate) struct ExitCode {
    inner: Result<(), ExecErr>,
    path: Option<PathBuf>,
}

#[derive(Error, Debug)]
enum ExecErr {
    #[error("python error {0}")]
    PyResult(#[from] PyErr),
    #[error("readline error {0}")]
    Readline(#[from] ReadlineError),
    #[error("io error {0}")]
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
                if let Some(path) = self.path {
                    println!("{}: {}", path.display(), e);
                } else {
                    println!("{}", e);
                }
                1.into()
            }
        }
    }
}

struct State {
    continuation: bool,
    on_error: bool,
}

#[derive(Completer, Helper, Hinter, Validator)]
struct MyHelper {
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
    state: State,
}

impl MyHelper {
    fn new() -> Self {
        Self {
            hinter: HistoryHinter::new(),
            state: State { continuation: false, on_error: false },
        }
    }
}

impl Highlighter for MyHelper {
    fn continuation_prompt<'p, 'b>(
        &self,
        _prompt: &'p str,
        default: bool,
    ) -> Option<Cow<'b, str>> {
        if default {
            Some(Borrowed(PROMPT2_OK))
        } else {
            Some(Borrowed(PROMPT2))
        }
    }
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            match (self.state.continuation, self.state.on_error) {
                (true, true) => Borrowed(PROMPT2_ERR),
                (true, false) => Borrowed(PROMPT2_OK),
                (false, true) => Borrowed(PROMPT1_ERR),
                (false, false) => Borrowed(PROMPT1_OK),
            }
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
    rl.set_helper(Some(MyHelper::new()));
    rl.set_auto_add_history(true);
    rl.bind_sequence(
        KeyEvent(KeyCode::Tab, Modifiers::NONE),
        EventHandler::Simple(Cmd::Indent(Movement::ForwardChar(4))),
    );
    rl.bind_sequence(
        KeyEvent(KeyCode::BackTab, Modifiers::NONE),
        EventHandler::Simple(Cmd::Dedent(Movement::BackwardChar(4))),
    );
    rl.bind_sequence(
        KeyEvent(KeyCode::Char('s'), Modifiers::CTRL),
        EventHandler::Simple(Cmd::Newline),
    );
    let mut code = String::new();
    let mut prompt = PROMPT1;
    let mut terminate_count: u8 = 0;
    Python::with_gil(|py| {
        py::init(py)?;
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
                    let (continuation, on_error) = if let Ok(true) =
                        py::is_incomplete_code(&compile_command, &code)
                    {
                        prompt = PROMPT1;
                        (true, None)
                    } else {
                        prompt = PROMPT2;
                        if let Err(e) = py.run_bound(&code, None, None) {
                            println!("{}", e);
                            code.clear();
                            (false, Some(true))
                        } else {
                            code.clear();
                            (false, Some(false))
                        }
                    };
                    if let Some(helper) = rl.helper_mut() {
                        helper.state.continuation = continuation;
                        if let Some(on_error) = on_error {
                            helper.state.on_error = on_error;
                        }
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

fn exec_file(path: &PathBuf, args: Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        let compile_command =
            PyModule::import_bound(py, "codeop")?.getattr("compile_command")?;
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut ping_pong_line_buffer = [String::new(), String::new()];
        let mut ping_pong_idx = true;
        let mut read_res = reader.read_line(&mut ping_pong_line_buffer[0]);
        let mut prompt = PROMPT1;
        let mut code = String::new();
        py::import_args(py, args)?;
        py::init(py)?;
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
                py.run_bound(&code, None, None)?;
                code.clear();
            }
        }
    })
}

fn quiet_exec_file(path: &PathBuf, args: Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        let mut file = File::open(path)?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        py::import_args(py, args)?;
        py::init(py)?;
        py.run_bound(buf.as_str(), None, None)?;
        Ok(())
    })
}

mod test {
    #[test]
    fn test_exec_file() {
        use super::*;
        use py::foo;
        pyo3::append_to_inittab!(foo);
        pyo3::prepare_freethreaded_python();
        exec_file(&PathBuf::from("tests/test1.py"), vec![]).expect("msg");
    }
    #[test]
    fn test_shell() {
        use super::*;
        use py::foo;
        pyo3::append_to_inittab!(foo);
        pyo3::prepare_freethreaded_python();
        run_shell().expect("msg");
    }
    #[test]
    fn test_pyo3() {
        use super::*;
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let result = py
                .eval_bound("[i * 10 for i in range(5)]", None, None)
                .map_err(|e| {
                    e.print_and_set_sys_last_vars(py);
                })
                .expect("msg");
            let res: Vec<i64> = result.extract().unwrap();
            assert_eq!(res, vec![0, 10, 20, 30, 40]);
        });
    }
}
