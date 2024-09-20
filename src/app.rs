use rustyline::highlight::syntect;

use crate::{
    args, py, PROMPT1, PROMPT1_ERR, PROMPT1_OK, PROMPT2, PROMPT2_OK, TERMINATE_N,
};
use pyo3::{
    types::{IntoPyDict, PyAnyMethods, PyModule},
    PyErr, Python,
};
use rustyline::{
    error::ReadlineError,
    highlight::{Highlighter, StyledBlock},
    hint::HistoryHinter,
    history::DefaultHistory,
    Cmd, Completer, Editor, EventHandler, Helper, Hinter, KeyCode, KeyEvent, Modifiers,
    Movement, Validator,
};
use std::{
    borrow::Cow::{self, Borrowed, Owned},
    fs::File,
    io::{BufRead, BufReader, Read},
    mem,
    path::PathBuf,
};
use syntect::{dumps::from_binary, highlighting::Theme, parsing::SyntaxSet};
use thiserror::Error;

include!(concat!(env!("OUT_DIR"), "/syntaxes_themes.rs"));

#[inline]
pub(super) fn run(args: args::Args) -> ExitCode {
    use py::foo;
    pyo3::append_to_inittab!(foo);
    pyo3::prepare_freethreaded_python();
    match args.mode {
        args::Mode::InteractiveShell => ExitCode { inner: run_shell(), path: None },
        args::Mode::ExecFile(py_args) => ExitCode {
            inner: if args.flag.quiet {
                quiet_exec_file(&py_args)
            } else {
                exec_file(&py_args)
            },
            path: Some((&py_args[0]).into()),
        },
        args::Mode::ExecModule(py_args) => {
            ExitCode { inner: run_module(&py_args), path: None }
        }
        args::Mode::Command(cmd, py_args) => {
            ExitCode { inner: run_command(&cmd, &py_args), path: None }
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
    #[inline]
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
    theme: Theme,
    syntax_set: SyntaxSet,
}

impl MyHelper {
    #[inline]
    fn new() -> Self {
        Self {
            hinter: HistoryHinter::new(),
            state: State { continuation: false, on_error: false },
            theme: from_binary(COMPRESSED_THEME),
            syntax_set: from_binary(COMPRESSED_SYNTAX_SET),
        }
    }
}

impl Highlighter for MyHelper {
    fn highlight_char(&self, line: &str, pos: usize, forced: bool) -> bool {
        // &line[pos..=pos];
        if line.len() == 0 {
            false
        } else {
            forced
                || line
                    .chars()
                    .nth(pos - 1)
                    .map_or(false, |c| " .:(){}[]><+-*/@=".contains(c))
        }
    }
    #[inline]
    fn highlight_lines<'l>(
        &self,
        lines: &'l str,
        _pos: usize,
    ) -> impl Iterator<Item = impl Iterator<Item = impl 'l + StyledBlock>> {
        use syntect::easy::HighlightLines;
        use syntect::highlighting::Style;
        let syntax = &self.syntax_set.syntaxes()[0];
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        lines.split('\n').map(move |l| {
            highlighter
                .highlight_line(l, &self.syntax_set)
                .unwrap_or(vec![(Style::default(), l)])
                .into_iter()
                .map(|(mut s, token)| {
                    s.background.a = 0;
                    (s, token)
                })
        })
    }
    #[inline]
    fn continuation_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        _prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(PROMPT2_OK)
        } else {
            Borrowed(PROMPT2)
        }
    }
    #[inline]
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            match (self.state.continuation, self.state.on_error) {
                (true, true) => Borrowed(PROMPT2_OK),
                (true, false) => Borrowed(PROMPT2_OK),
                (false, true) => Borrowed(PROMPT1_ERR),
                (false, false) => Borrowed(PROMPT1_OK),
            }
        } else {
            Borrowed(prompt)
        }
    }
    #[inline]
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned(format!("\x1b[1m{hint}\x1b[m"))
    }
}

#[inline]
fn run_shell() -> Result<(), ExecErr> {
    let mut rl = Editor::<MyHelper, DefaultHistory>::new()?;
    rl.set_helper(Some(MyHelper::new()));
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
                    match line.as_str() {
                        "clear" => {
                            rl.clear_screen()?;
                            if let Some(helper) = rl.helper_mut() {
                                helper.state.on_error = false;
                            }
                            continue;
                        }
                        "exit" => {
                            println!("\nExiting...");
                            return Ok(());
                        }
                        _ => (),
                    }
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
                            rl.add_history_entry(mem::take(&mut code))?;
                            (false, Some(true))
                        } else {
                            rl.add_history_entry(mem::take(&mut code))?;
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

#[inline]
fn run_module(py_args: &Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        py::import_args(py, py_args)?;
        let module = py_args.first().unwrap();
        let runpy = PyModule::import_bound(py, "runpy")?;
        match runpy.call_method(
            "run_module",
            (module,),
            Some(
                &[("run_name", "__main__"), ("alter_sys", "true")].into_py_dict_bound(py),
            ),
        ) {
            Ok(_) => Ok(()),
            Err(e) => {
                if "SystemExit: 0" == &(e.to_string()) {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    })
}

#[inline]
fn run_command(cmd: &str, py_args: &Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        py::import_args(py, py_args)?;
        py::init(py)?;
        py.run_bound(cmd, None, None).map_err(Into::into)
    })
}

#[inline]
fn exec_file(py_args: &Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        let compile_command =
            PyModule::import_bound(py, "codeop")?.getattr("compile_command")?;
        let file_path = py_args.first().unwrap();
        let file = File::open(file_path)?;
        let mut reader = BufReader::new(file);
        let mut ping_pong_line_buffer = [String::new(), String::new()];
        let mut ping_pong_idx = true;
        let mut read_res = reader.read_line(&mut ping_pong_line_buffer[0]);
        let mut prompt = PROMPT1;
        let mut code = String::new();
        py::import_args(py, py_args)?;
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

#[inline]
fn quiet_exec_file(py_args: &Vec<String>) -> Result<(), ExecErr> {
    Python::with_gil(|py| {
        let file_path = py_args.first().unwrap();
        let mut file = File::open(file_path)?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        py::import_args(py, py_args)?;
        // TODO:
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
        exec_file(&vec!["tests/test1.py".into()]).expect("msg");
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
    fn test_cmd() {
        use super::*;
        use py::foo;
        pyo3::append_to_inittab!(foo);
        pyo3::prepare_freethreaded_python();
        run_command(
            "import sys;print(sys.argv)".into(),
            &vec!["-c".into(), "11".into(), "22".into()],
        )
        .expect("msg");
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
