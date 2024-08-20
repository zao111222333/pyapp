use core::fmt;
use std::ffi::OsString;
use thiserror::Error;

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Mode {
    /// Start in interactive shell mode
    #[default]
    InteractiveShell,
    /// Execute a file with arguments
    ExecFile(Vec<String>),
    /// Execute a module with arguments
    ExecModule(Vec<String>),
    /// Execute a command
    Command(String, Vec<String>),
}

#[derive(Error, Debug)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArgsError {
    #[error("Unknow Flag '-{0}'")]
    UnknowShort(char),
    #[error("Unknow Flag '--{0}'")]
    UnknowLong(String),
    #[error("ExpectValue {0}")]
    ExpectValue(Arg),
    #[error("OsString {0:?}")]
    OsString(OsString),
}

#[derive(Debug, Default)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Flag {
    /// execute in quiet mode (effect in file mode)
    // -q
    pub(crate) quiet: bool,
    /// [PYTHON] isolate Python from the user's environment (implies -E and -s)
    // -I
    pub(crate) isolate: bool,
    /// [PYTHON] don't add user site directory to sys.path; also PYTHONNOUSERSITE
    // -s
    pub(crate) ignore_site: bool,
    /// [PYTHON] ignore PYTHON* environment variables (such as PYTHONPATH)
    // -E
    pub(crate) ignore_env: bool,
}
#[derive(Debug, Default)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Args {
    pub(crate) mode: Mode,
    pub(crate) flag: Flag,
}

#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Arg {
    // -m
    Module,
    // -c
    Command,
}

impl fmt::Display for Arg {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Module => f.write_str("-m"),
            Arg::Command => f.write_str("-c"),
        }
    }
}

impl Args {
    fn version() -> ! {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"),);
        std::process::exit(0)
    }
    fn help(e: Option<ArgsError>) -> ! {
        let code = if let Some(e) = e {
            println!("Error: {e}\n");
            2
        } else {
            println!("An example application\n");
            0
        };
        println!("Usage: {} [option] ... [-c cmd | -m mod | file | -] [arg] ...

Arguments:
    [file]    [PYTHON] program read from script file
    [arg] ... [PYTHON] arguments passed to program in sys.argv[1:]
    [ - ]     [PYTHON] program read from stdin (default; interactive mode if a tty)

Options:
    -q, --quiet    execute in quiet mode (effect in file mode)
    -I             [PYTHON] isolate Python from the user's environment (implies -E and -s)
    -s             [PYTHON] don't add user site directory to sys.path; also PYTHONNOUSERSITE
    -E             [PYTHON] ignore PYTHON* environment variables (such as PYTHONPATH)
    -m <mod>       [PYTHON] run library module as a script (terminates option list)
    -c <cmd>       [PYTHON] program passed in as string (terminates option list)
    -h, --help     Print help
    -V, --version  Print version", env!("CARGO_PKG_NAME"));
        std::process::exit(code)
    }
    fn match_long(
        s: &str,
        flag: &mut Flag,
        last_arg: &mut Option<Arg>,
    ) -> Result<(), ArgsError> {
        if let Some(last) = last_arg {
            Err(ArgsError::ExpectValue(*last))
        } else {
            match s {
                "quiet" => {
                    flag.quiet = true;
                    *last_arg = None;
                    Ok(())
                }
                "help" => {
                    Self::help(None);
                }
                "version" => {
                    Self::version();
                }
                _ => Err(ArgsError::UnknowLong(s.to_owned())),
            }
        }
    }

    fn match_short(
        c: char,
        flag: &mut Flag,
        last_arg: &mut Option<Arg>,
    ) -> Result<(), ArgsError> {
        if let Some(last) = last_arg {
            Err(ArgsError::ExpectValue(*last))
        } else {
            match c {
                'm' => {
                    *last_arg = Some(Arg::Module);
                    Ok(())
                }
                'c' => {
                    *last_arg = Some(Arg::Command);
                    Ok(())
                }
                'q' => {
                    flag.quiet = true;
                    *last_arg = None;
                    Ok(())
                }
                'I' => {
                    flag.isolate = true;
                    *last_arg = None;
                    Ok(())
                }
                's' => {
                    flag.ignore_site = true;
                    *last_arg = None;
                    Ok(())
                }
                'E' => {
                    flag.ignore_env = true;
                    *last_arg = None;
                    Ok(())
                }
                'h' => {
                    Self::help(None);
                }
                'V' => {
                    Self::version();
                }
                _ => Err(ArgsError::UnknowShort(c)),
            }
        }
    }
    pub(crate) fn parse() -> Self {
        Self::parse_from(std::env::args_os()).unwrap_or_else(|e| Self::help(Some(e)))
    }
    fn parse_from<I, T>(itr: I) -> Result<Self, ArgsError>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut last_arg = None;
        let mut out = Args::default();
        let mut iter = itr.into_iter().skip(1);
        while let Some(arg) = iter.next() {
            let arg_os_str = Into::<OsString>::into(arg);
            let arg_str = if let Some(s) = arg_os_str.to_str() {
                s
            } else {
                return Err(ArgsError::OsString(arg_os_str));
            };
            let mut chars: std::str::Chars = arg_str.chars();
            match chars.next() {
                None => continue,
                Some('-') => match chars.next() {
                    None => {
                        return Err(ArgsError::UnknowShort('\0'));
                    }
                    Some('-') => {
                        Self::match_long(&arg_str[2..], &mut out.flag, &mut last_arg)?;
                    }
                    Some(c) => {
                        Self::match_short(c, &mut out.flag, &mut last_arg)?;
                        while let Some(c) = chars.next() {
                            Self::match_short(c, &mut out.flag, &mut last_arg)?;
                        }
                    }
                },
                _ => match last_arg {
                    None => {
                        out.mode = Mode::ExecFile(vec![arg_str.into()]);
                        break;
                    }
                    Some(Arg::Module) => {
                        out.mode = Mode::ExecModule(vec![arg_str.into()]);
                        break;
                    }
                    Some(Arg::Command) => {
                        out.mode = Mode::Command(arg_str.into(), vec!["-c".to_owned()]);
                        break;
                    }
                },
            }
        }
        match &mut out.mode {
            Mode::InteractiveShell => {
                if let Some(last) = last_arg {
                    Err(ArgsError::ExpectValue(last))
                } else {
                    Ok(out)
                }
            }
            Mode::ExecFile(args) | Mode::ExecModule(args) | Mode::Command(_, args) => {
                if let Err(e) = iter.try_for_each(|arg| {
                    Into::<OsString>::into(arg).into_string().map(|s| args.push(s))
                }) {
                    Err(ArgsError::OsString(e))
                } else {
                    Ok(out)
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn short() {
        assert_eq!(
            Args::parse_from({
                let args: &[&str] = &[];
                args
            }),
            Ok(Args {
                mode: Mode::InteractiveShell,
                flag: Flag::default()
            })
        );
        assert_eq!(
            Args::parse_from(&["-qI"]),
            Ok(Args {
                mode: Mode::InteractiveShell,
                flag: {
                    let mut f = Flag::default();
                    f.quiet = true;
                    f.isolate = true;
                    f
                }
            })
        );
        assert_eq!(
            Args::parse_from(&["-qc", "print(1);", "arg1", "arg2"]),
            Ok(Args {
                mode: Mode::Command(
                    "print(1);".into(),
                    vec!["-c".into(), "arg1".into(), "arg2".into()]
                ),
                flag: {
                    let mut f = Flag::default();
                    f.quiet = true;
                    f
                }
            })
        );
        assert_eq!(
            Args::parse_from(&["-Iqs", "run.py", "arg1", "arg2"]),
            Ok(Args {
                mode: Mode::ExecFile(vec!["run.py".into(), "arg1".into(), "arg2".into()]),
                flag: {
                    let mut f = Flag::default();
                    f.quiet = true;
                    f.isolate = true;
                    f.ignore_site = true;
                    f
                }
            })
        );
        assert_eq!(Args::parse_from(&["-c"]), Err(ArgsError::ExpectValue(Arg::Command)));
        assert_eq!(Args::parse_from(&["-m"]), Err(ArgsError::ExpectValue(Arg::Module)));
        assert_eq!(
            Args::parse_from(&["-cq", "arg1", "arg2"]),
            Err(ArgsError::ExpectValue(Arg::Command))
        );
    }
}
