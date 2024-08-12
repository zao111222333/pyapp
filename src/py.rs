use pyo3::{prelude::*, types::PyList};

#[pyfunction]
fn add_one(x: i64) -> i64 {
    x + 1
}

#[pyfunction]
fn exit(code: u8) {
    println!("exit..");
    std::process::exit(code.into());
}

#[pymodule]
pub(super) fn foo(foo_module: &Bound<'_, PyModule>) -> PyResult<()> {
    foo_module.add_function(wrap_pyfunction!(add_one, foo_module)?)?;
    foo_module.add_function(wrap_pyfunction!(exit, foo_module)?)?;
    Ok(())
}

pub(super) fn import_args(py: Python, args: Vec<String>) -> PyResult<()> {
    PyModule::import_bound(py, "sys")?.setattr("argv", PyList::new_bound(py, args))
}

pub(super) fn is_incomplete_code(
    compile_command: &Bound<PyAny>,
    code: &str,
) -> PyResult<bool> {
    let result = compile_command.call1((code, "<input>", "single"))?;
    Ok(result.is_none())
}
