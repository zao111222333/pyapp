use crate::{
    args, py, BLANK_COLOR, BRACKET_COLORS, CLASS_COLOR, COMMENT_COLOR, FUNCTION_COLOR,
    KEY1_COLOR, KEY2_COLOR, PROMPT1, PROMPT1_ERR, PROMPT1_OK, PROMPT2, PROMPT2_OK,
    STRING_COLOR, SYMBOL_COLOR, TERMINATE_N, UNKNOWN_COLOR,
};
use anstyle::Style;
use pyo3::{
    types::{IntoPyDict, PyAnyMethods, PyModule},
    PyErr, Python,
};
use ruff_python_parser::{
    LexicalErrorType, ParseError, ParseErrorType, Token, TokenKind,
};
use rustyline::{
    completion::Completer,
    config::Configurer,
    error::ReadlineError,
    highlight::{Highlighter, StyledBlock},
    hint::Hinter,
    history::DefaultHistory,
    validate::{ValidationContext, ValidationResult, Validator},
    Cmd, Editor, EventHandler, Helper, KeyCode, KeyEvent, Modifiers, Movement,
};
use std::{
    borrow::Cow::{self, Borrowed, Owned},
    fs::File,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
};
use thiserror::Error;

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

// #[derive(Completer, Helper, Hinter, Validator)]
struct MyHelper {
    tokens: Vec<Token>,
    errors: Vec<ParseError>,
    bracket_level_diff: i32,
    need_render: bool,
    on_error: bool,
}

impl Validator for MyHelper {
    fn validate(
        &mut self,
        _ctx: &mut ValidationContext,
    ) -> rustyline::Result<ValidationResult> {
        let mut indent = self.bracket_level_diff.try_into().unwrap_or(0);
        let mut incomplete = false;
        let mut tokens_rev = self.tokens.iter().rev();
        while let Some(token) = tokens_rev.next() {
            let (kind, range) = token.as_tuple();
            match kind {
                TokenKind::Dedent => {
                    indent += 1;
                    incomplete = true;
                }
                TokenKind::NonLogicalNewline | TokenKind::Newline => {
                    if incomplete {
                        incomplete = range.len().to_u32() == 0
                    }
                    break;
                }
                _ => break,
            }
        }
        for error in &self.errors {
            match &error.error {
                ParseErrorType::OtherError(s) => {
                    if s.starts_with("Expected an indented") {
                        incomplete = true;
                        indent += 1;
                        break;
                    }
                }
                ParseErrorType::Lexical(
                    LexicalErrorType::Eof | LexicalErrorType::LineContinuationError,
                ) => {
                    incomplete = true;
                    break;
                }
                _ => {}
            }
        }
        if incomplete {
            Ok(ValidationResult::Incomplete(indent * 2))
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}
impl Completer for MyHelper {
    type Candidate = String;
}
impl Hinter for MyHelper {
    type Hint = String;
}

impl Helper for MyHelper {
    fn update_after_edit(&mut self, line: &str, _pos: usize, _forced_refresh: bool) {
        use ruff_python_parser::{parse_unchecked, Mode};
        let (_, tokens, errors) = parse_unchecked(line, Mode::Module).into_tuple();
        self.bracket_level_diff =
            tokens.iter().fold(0, |level, token| match token.kind() {
                TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => level + 1,
                TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => level - 1,
                _ => level,
            });
        self.tokens = tokens.into();
        self.errors = errors;
        self.need_render = true;
    }
}

impl MyHelper {
    #[inline]
    fn new() -> Self {
        Self {
            on_error: false,
            need_render: true,
            bracket_level_diff: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl Highlighter for MyHelper {
    fn highlight_char(&mut self, _line: &str, _pos: usize, _forced: bool) -> bool {
        self.need_render
    }
    #[inline]
    fn highlight_line<'l>(
        &mut self,
        line: &'l str,
        _pos: usize,
    ) -> impl Iterator<Item = impl 'l + StyledBlock> {
        self.need_render = false;
        let tokens = &self.tokens;
        let bracket_level_diff = self.bracket_level_diff;
        let mut last_end = 0;
        let mut bracket_level: i32 = 0;
        let mut last_kind = TokenKind::Name;
        tokens.iter().enumerate().flat_map(move |(idx, token)| {
            let (kind, range) = token.as_tuple();
            let term = match kind {
                TokenKind::Newline | TokenKind::NonLogicalNewline => {
                    if range.len().to_u32() == 0 {
                        ""
                    } else {
                        PROMPT2_OK
                    }
                }
                _ => &line[range],
            };
            let style = match kind {
                TokenKind::Name => match last_kind {
                    // function
                    TokenKind::Def => Style::new().fg_color(Some(FUNCTION_COLOR)),
                    TokenKind::Class => Style::new().fg_color(Some(CLASS_COLOR)),
                    _ => match term {
                        "self" | "super" => Style::new().fg_color(Some(KEY1_COLOR)),
                        _ => {
                            if term.chars().all(|c| c.is_ascii_uppercase()) {
                                Style::new().fg_color(Some(KEY1_COLOR))
                            } else {
                                if let Some(next_token) = tokens.get(idx + 1) {
                                    match next_token.kind() {
                                        TokenKind::Lpar
                                        | TokenKind::Lsqb
                                        | TokenKind::Lbrace => {
                                            Style::new().fg_color(Some(FUNCTION_COLOR))
                                        }
                                        _ => Style::new().fg_color(Some(BLANK_COLOR)),
                                    }
                                } else {
                                    Style::new().fg_color(Some(BLANK_COLOR))
                                }
                            }
                        }
                    },
                },
                TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => {
                    let style = Style::new().fg_color(Some(
                        if bracket_level_diff <= bracket_level + 1 {
                            TryInto::<usize>::try_into(bracket_level)
                                .map_or(UNKNOWN_COLOR, |level| {
                                    BRACKET_COLORS[level % BRACKET_COLORS.len()]
                                })
                        } else {
                            UNKNOWN_COLOR
                        },
                    ));
                    bracket_level += 1;
                    style
                }
                TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => {
                    bracket_level -= 1;
                    let style = Style::new().fg_color(Some(
                        TryInto::<usize>::try_into(bracket_level)
                            .map_or(UNKNOWN_COLOR, |level| {
                                BRACKET_COLORS[level % BRACKET_COLORS.len()]
                            }),
                    ));
                    style
                }
                TokenKind::From
                | TokenKind::Import
                | TokenKind::Def
                | TokenKind::Class
                | TokenKind::Equal
                | TokenKind::EqEqual
                | TokenKind::NotEqual
                | TokenKind::LessEqual
                | TokenKind::GreaterEqual
                | TokenKind::DoubleStarEqual
                | TokenKind::PlusEqual
                | TokenKind::MinusEqual
                | TokenKind::StarEqual
                | TokenKind::SlashEqual
                | TokenKind::PercentEqual
                | TokenKind::AmperEqual
                | TokenKind::VbarEqual
                | TokenKind::CircumflexEqual
                | TokenKind::LeftShiftEqual
                | TokenKind::RightShiftEqual
                | TokenKind::DoubleSlash
                | TokenKind::DoubleSlashEqual
                | TokenKind::ColonEqual
                | TokenKind::At
                | TokenKind::AtEqual
                | TokenKind::Elif
                | TokenKind::Else
                | TokenKind::For
                | TokenKind::If
                | TokenKind::In
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Vbar
                | TokenKind::Amper
                | TokenKind::Less
                | TokenKind::Greater
                | TokenKind::Percent
                | TokenKind::Tilde
                | TokenKind::CircumFlex
                | TokenKind::LeftShift
                | TokenKind::RightShift
                | TokenKind::Dot
                | TokenKind::DoubleStar
                | TokenKind::As
                | TokenKind::Assert
                | TokenKind::Async
                | TokenKind::Await
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Del
                | TokenKind::Except
                | TokenKind::Global
                | TokenKind::Is
                | TokenKind::Lambda
                | TokenKind::Finally
                | TokenKind::Nonlocal
                | TokenKind::Not
                | TokenKind::Pass
                | TokenKind::Raise
                | TokenKind::Return
                | TokenKind::Try
                | TokenKind::While
                | TokenKind::With
                | TokenKind::Yield
                | TokenKind::Case
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Match => Style::new().fg_color(Some(KEY2_COLOR)),
                TokenKind::String
                | TokenKind::FStringStart
                | TokenKind::FStringMiddle
                | TokenKind::FStringEnd => Style::new().fg_color(Some(STRING_COLOR)),
                TokenKind::Int
                | TokenKind::Float
                | TokenKind::Complex
                | TokenKind::Ellipsis
                | TokenKind::True
                | TokenKind::False
                | TokenKind::None
                | TokenKind::Type => Style::new().fg_color(Some(KEY1_COLOR)),
                TokenKind::Comment => Style::new().fg_color(Some(COMMENT_COLOR)).italic(),
                TokenKind::Comma
                | TokenKind::Unknown
                | TokenKind::IpyEscapeCommand
                | TokenKind::Exclamation
                | TokenKind::Colon => Style::new().fg_color(Some(BLANK_COLOR)),
                TokenKind::Indent
                | TokenKind::Dedent
                | TokenKind::Newline
                | TokenKind::NonLogicalNewline
                | TokenKind::EndOfFile => Style::new(),
                TokenKind::Semi | TokenKind::Question | TokenKind::Rarrow => {
                    Style::new().fg_color(Some(SYMBOL_COLOR)).italic()
                }
            };
            last_kind = kind;
            let out =
                core::iter::once((style, &line[last_end..range.start().to_usize()]))
                    .chain(core::iter::once((style, term)));
            last_end = range.end().to_usize();
            out
        })
    }
    #[inline]
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s mut self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            if self.on_error {
                Borrowed(PROMPT1_ERR)
            } else {
                Borrowed(PROMPT1_OK)
            }
        } else {
            Borrowed(prompt)
        }
    }
    #[inline]
    fn highlight_hint<'h>(&mut self, hint: &'h str) -> Cow<'h, str> {
        Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }
}

#[inline]
fn run_shell() -> Result<(), ExecErr> {
    let mut rl = Editor::<MyHelper, DefaultHistory>::new(MyHelper::new())?;
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
    let mut terminate_count: u8 = 0;
    Python::with_gil(|py| {
        py::init(py)?;
        loop {
            let readline = rl.readline(PROMPT1);
            match readline {
                Ok(input) => {
                    match input.trim() {
                        "clear()" => {
                            rl.clear_screen()?;
                            rl.helper_mut().on_error = false;
                            continue;
                        }
                        "exit()" => {
                            println!("\nExiting...");
                            return Ok(());
                        }
                        _ => (),
                    }
                    terminate_count = 0;
                    if let Err(e) = py.run_bound(&input, None, None) {
                        println!("{}", e);
                        rl.helper_mut().on_error = true;
                    } else {
                        rl.helper_mut().on_error = false;
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
