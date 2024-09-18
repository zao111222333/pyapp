use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use syntect::dumps::dump_binary;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSetBuilder;
fn main() {
    // let ts = ThemeSet::load_defaults();
    // let theme = &ts.themes["base16-ocean.dark"];
    let theme = ThemeSet::get_theme("assets/themes/Base16 Ocean Dark.tmTheme").unwrap();
    // let theme = ThemeSet::get_theme("assets/themes/Dracula.tmTheme").unwrap();
    // let theme = ThemeSet::get_theme("assets/themes/base16-256.tmTheme").unwrap();
    let mut builder = SyntaxSetBuilder::new();
    builder.add_from_folder("assets/syntax", true).unwrap();
    // let ps = SyntaxSet::load_defaults_newlines();
    let ps = builder.build();

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest_path = out_dir.join("syntaxes_themes.rs");
    let mut f = File::create(&dest_path).unwrap();
    writeln!(f, "pub static COMPRESSED_THEME: &[u8] = &{:?};", dump_binary(&theme))
        .unwrap();
    writeln!(f, "pub static COMPRESSED_SYNTAX_SET: &[u8] = &{:?};", dump_binary(&ps))
        .unwrap();
}
