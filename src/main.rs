use pyo3::prelude::*;
use rustyline::{config::Configurer,error::ReadlineError,Cmd, DefaultEditor, EventHandler, KeyCode, KeyEvent, Modifiers};
use thiserror::Error;

#[pyfunction]
fn add_one(x: i64) -> i64 {
    x + 1
}

static mut EXIT_CODE: Option<u8> = None;

#[pyfunction]
fn exit(code: u8) {
    unsafe {
        EXIT_CODE = Some(code);
    }
}

#[pymodule]
fn foo(foo_module: &Bound<'_, PyModule>) -> PyResult<()> {
    foo_module.add_function(wrap_pyfunction!(add_one, foo_module)?)?;
    foo_module.add_function(wrap_pyfunction!(exit, foo_module)?)?;
    Ok(())
}

#[derive(Error, Debug)]
enum ExitCode {
    #[error("data store disconnected")]
    Exit(u8),
    #[error("data store disconnected")]
    PyResult(#[from] PyErr),
    #[error("data store disconnected")]
    Readline(#[from] ReadlineError),
}

impl std::process::Termination for ExitCode {
    fn report(self) -> std::process::ExitCode {
        match self {
            ExitCode::Exit(code) => {
                println!("exit");
                code.into()
            },
            ExitCode::PyResult(e) => {
                println!("{}",e);
                1.into()
            },
            ExitCode::Readline(e) => {
                println!("{}",e);
                1.into()
            },
        }
    }
}

fn is_incomplete_code(compile_command: &Bound<PyAny>, code: &str) -> PyResult<bool> {
    let result = compile_command.call1((code, "<input>", "single"))?;
    Ok(result.is_none())
}

fn run_shell() -> Result<(), ExitCode> {
    let mut rl = DefaultEditor::new()?;
    rl.set_auto_add_history(true);
    rl.bind_sequence(
        KeyEvent(KeyCode::Char('s'), Modifiers::CTRL),
        EventHandler::Simple(Cmd::Newline),
    );
    let mut code = String::new();
    let terminate_n: u8 = 2;
    let mut terminate_count: u8  = 0;

    let prompt1 = "\x1b[1;32mmy-app > \x1b[m";
    let prompt2 = "\x1b[1;32m ..... > \x1b[m";
    let mut prompt = prompt1;
    Python::with_gil(|py| {
        let compile_command = PyModule::import_bound(py, "codeop")?.getattr("compile_command")?;
        loop {
            unsafe { if let Some(code) = EXIT_CODE { return Err(ExitCode::Exit(code)); } };
            let readline = rl.readline(prompt);
            match readline {
                Ok(line) => {
                    terminate_count = 0;
                    if !code.is_empty(){ code += "\n"; }
                    code += &line;
                    if let Ok(true) = is_incomplete_code(&compile_command,&code) {
                        prompt = prompt2;
                    } else {
                        prompt = prompt1;
                        if let Err(e) = py.run_bound(&code, None, None) {
                            println!("{}",e);
                        }
                        code.clear();
                    }
                },
                Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                    if terminate_count >= terminate_n {
                        println!("\nExiting...");
                        return Ok(());
                    }
                    println!("Need {} interrupt to exit..", terminate_n - terminate_count);
                    terminate_count += 1;
                },
                Err(err) => {
                    println!("Error: {:?}", err);
                    return Err(ExitCode::Readline(err));
                }
            }
        };
    })
}

fn main() -> ExitCode {
    pyo3::append_to_inittab!(foo);
    pyo3::prepare_freethreaded_python();
    match run_shell(){
        Ok(_) => ExitCode::Exit(0),
        Err(e) => e,
    }
}