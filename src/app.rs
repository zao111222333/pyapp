use rustyline::highlight::syntect::{self, highlighting::Color};

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

struct BracketSplitter<'a> {
    bytes: core::slice::Iter<'a, u8>,
    pos: usize,
    level: i16,
}

impl<'a> BracketSplitter<'a> {
    fn new<'b, const N: usize, C>(
        s: &'a str,
        cursor_pos: usize,
        colors: &'b [C; N],
        color_invalid: &'b C,
        color_focus: &'b C,
    ) -> impl 'b + Iterator<Item = (usize, (&'b C, Option<&'b C>))> {
        let mut focus_l_idx = None;
        let mut focus_r_idx = None;
        let mut open_bracket_idx = Vec::new();
        let vec = Self { bytes: s.as_bytes().into_iter(), pos: 0, level: 0 }
            .into_iter()
            .enumerate()
            .map(|(idx, (pos, is_open, level))| {
                match (focus_l_idx, focus_r_idx) {
                    (None, None) => {
                        if pos >= cursor_pos {
                            if is_open {
                                if pos == cursor_pos {
                                    focus_l_idx = Some(idx);
                                    open_bracket_idx.push(idx);
                                } else {
                                    focus_l_idx = open_bracket_idx.last().copied();
                                }
                            } else {
                                if let Some(_focus_l_idx) = open_bracket_idx.pop() {
                                    focus_l_idx = Some(_focus_l_idx);
                                    focus_r_idx = Some(idx);
                                };
                            }
                        } else {
                            if is_open {
                                open_bracket_idx.push(idx);
                            } else {
                                open_bracket_idx.pop();
                            }
                        }
                    }
                    (None, Some(_)) => {
                        unreachable!()
                    }
                    (Some(_), None) => {
                        if is_open {
                            open_bracket_idx.push(pos);
                        } else {
                            if focus_l_idx == open_bracket_idx.pop() {
                                focus_r_idx = Some(idx);
                            }
                        }
                    }
                    (Some(_), Some(_)) => {}
                }
                (pos, level)
            })
            .collect::<Vec<_>>();
        let min = if let (Some((_, first_level)), Some((_, last_level))) =
            (vec.first(), vec.last())
        {
            0.max(*last_level).max(*first_level)
        } else {
            0
        };
        let m = vec.into_iter().enumerate().map(move |(idx, (pos, level))| {
            (pos, {
                let color_fb = if level >= min {
                    colors.get(level as usize % N).unwrap_or(color_invalid)
                } else {
                    color_invalid
                };
                let color_bg = if Some(idx) == focus_l_idx || Some(idx) == focus_r_idx {
                    Some(color_focus)
                } else {
                    None
                };
                (color_fb, color_bg)
            })
        });
        m
    }
}

impl<'a> Iterator for BracketSplitter<'a> {
    type Item = (usize, bool, i16);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(c) = self.bytes.next() {
            match c {
                b'(' | b'{' | b'[' => {
                    self.level += 1;
                    let out = (self.pos, true, self.level);
                    self.pos += 1;
                    return Some(out);
                }
                b')' | b'}' | b']' => {
                    let out = (self.pos, false, self.level);
                    self.level -= 1;
                    self.pos += 1;
                    return Some(out);
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
        None
    }
}

impl Highlighter for MyHelper {
    fn highlight_char(&self, line: &str, pos: usize, forced: bool) -> bool {
        if line.len() == 0 {
            false
        } else {
            let bytes = line.as_bytes();
            let c1 = bytes.get(pos - 1);
            let c2 = bytes.get(pos + 1);
            forced
                || c1.map_or(false, |c| {
                    matches!(
                        c,
                        b' ' | b'.'
                            | b':'
                            | b'('
                            | b')'
                            | b'['
                            | b']'
                            | b'{'
                            | b'}'
                            | b'>'
                            | b'<'
                            | b'+'
                            | b'-'
                            | b'*'
                            | b'/'
                            | b'@'
                            | b'='
                    )
                })
                || c2.map_or(false, |c| {
                    matches!(c, b' ' | b'(' | b')' | b'[' | b']' | b'{' | b'}')
                })
        }
    }
    #[inline]
    fn highlight_lines<'l>(
        &self,
        lines: &'l str,
        pos: usize,
    ) -> impl Iterator<Item = impl Iterator<Item = impl 'l + StyledBlock>> {
        use syntect::easy::HighlightLines;
        use syntect::highlighting::Style;
        let syntax = &self.syntax_set.syntaxes()[0];
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        let mut bgn_pos = 0;
        let mut brackets = BracketSplitter::new(
            lines,
            pos,
            &[
                Color { r: 0xFF, g: 0xFF, b: 0x00, a: 0xFF },
                Color { r: 0xFF, g: 0x00, b: 0xFF, a: 0xFF },
                Color { r: 0x00, g: 0xFF, b: 0xFF, a: 0xFF },
            ],
            &Color { r: 0xFF, g: 0x00, b: 0x00, a: 0xFF },
            &Color { r: 0x00, g: 0xFF, b: 0x00, a: 0xFF },
        );
        let mut bracket = brackets.next();
        lines.split('\n').map(move |l| {
            let iter = highlighter
                .highlight_line(l, &self.syntax_set)
                .unwrap_or(vec![(Style::default(), l)])
                .into_iter()
                .flat_map(|(mut s, token)| {
                    s.background.a = 0;
                    let end_pos = bgn_pos + token.len();
                    let mut v = Vec::new();
                    while let Some((bracket_pos, (color_fb, color_bg))) = bracket {
                        if bgn_pos <= bracket_pos && end_pos > bracket_pos {
                            let shifted_pos = bracket_pos - bgn_pos;
                            v.push((s, &token[0..shifted_pos]));
                            v.push((
                                {
                                    let mut _s = s;
                                    _s.foreground = *color_fb;
                                    if let Some(color_bg) = color_bg {
                                        _s.background = *color_bg;
                                    }
                                    _s
                                },
                                &token[shifted_pos..=shifted_pos],
                            ));
                            bgn_pos = bracket_pos + 1;
                            bracket = brackets.next();
                        } else {
                            break;
                        }
                    }
                    v.push((s, &lines[bgn_pos..end_pos]));
                    bgn_pos = end_pos;
                    v.into_iter()
                })
                .collect::<Vec<_>>();
            bgn_pos += 1;
            iter.into_iter()
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
        Owned(format!("\x1b[90m{hint}\x1b[0m"))
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
