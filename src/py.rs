use core::sync::atomic::Ordering;
use pyo3::{prelude::*, types::PyList};
use std::sync::atomic::{AtomicBool, AtomicU8};

const PY_FOO: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/py/utils/foo.py"));
const PY_INIT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/py/init.py"));

pub static CLEAR: AtomicBool = AtomicBool::new(false);
pub static EXIT: AtomicBool = AtomicBool::new(false);
pub static EXIT_CODE: AtomicU8 = AtomicU8::new(0);

#[pyfunction]
fn add_one(x: i64) -> i64 {
    x + 1
}

#[pyfunction]
fn clear() {
    CLEAR.store(true, Ordering::Relaxed);
}

#[pyfunction]
fn exit(code: u8) {
    EXIT.store(true, Ordering::Relaxed);
    EXIT_CODE.store(code, Ordering::Relaxed);
}

#[pyfunction]
fn loading() -> PyResult<()> {
    // https://github.com/clitic/kdam/blob/main/kdam/examples/rich.rs
    use kdam::{term, term::Colorizer, tqdm, BarExt, Column, RichProgress, Spinner};
    use std::io::{stderr, IsTerminal};
    term::init(stderr().is_terminal());
    term::hide_cursor()?;

    let mut pb = RichProgress::new(
        tqdm!(total = 231231231, unit_scale = true, unit_divisor = 1024, unit = "B"),
        vec![
            Column::Spinner(Spinner::new(
                &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
                80.0,
                1.0,
            )),
            Column::Text("[bold blue]?".to_owned()),
            Column::Animation,
            Column::Percentage(1),
            Column::Text("•".to_owned()),
            Column::CountTotal,
            Column::Text("•".to_owned()),
            Column::Rate,
            Column::Text("•".to_owned()),
            Column::RemainingTime,
        ],
    );

    pb.write("download will begin in 2 seconds".colorize("bold red"))?;

    while pb.pb.elapsed_time() <= 2.0 {
        pb.refresh()?;
    }

    pb.replace(1, Column::Text("[bold blue]docker.exe".to_owned()));
    pb.write("downloading docker.exe".colorize("bold cyan"))?;

    let total_size = 231231231;
    let mut downloaded = 0;

    while downloaded < total_size {
        let new = std::cmp::min(downloaded + 2203211, total_size);
        downloaded = new;
        pb.update_to(new)?;
        std::thread::sleep(std::time::Duration::from_millis(12));
    }

    pb.write("downloaded docker.exe".colorize("bold green"))?;
    eprintln!();
    term::show_cursor()?;
    Ok(())
}

#[pymodule]
pub(super) fn foo(foo_module: &Bound<'_, PyModule>) -> PyResult<()> {
    foo_module.add_function(wrap_pyfunction!(add_one, foo_module)?)?;
    foo_module.add_function(wrap_pyfunction!(exit, foo_module)?)?;
    foo_module.add_function(wrap_pyfunction!(clear, foo_module)?)?;
    foo_module.add_function(wrap_pyfunction!(loading, foo_module)?)?;
    Ok(())
}

pub(super) fn import_args(py: Python, py_args: &Vec<String>) -> PyResult<()> {
    PyModule::import_bound(py, "sys")?.setattr("argv", PyList::new_bound(py, py_args))
}

pub(super) fn init(py: Python) -> PyResult<()> {
    PyModule::from_code_bound(py, PY_FOO, "utils/foo.py", "utils.foo")?;
    PyModule::from_code_bound(py, PY_INIT, "init.py", "")?;
    Ok(())
}

pub(super) fn is_incomplete_code(
    compile_command: &Bound<PyAny>,
    code: &str,
) -> PyResult<bool> {
    let result = compile_command.call1((code, "<input>", "single"))?;
    Ok(result.is_none())
}
