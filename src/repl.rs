// Copyright 2018 the Deno authors. All rights reserved. MIT license.
extern crate rustyline;
use rustyline::Editor;
use rustyline::error::ReadlineError::Interrupted;

use std::error::Error;
use msg::ErrorKind;

use errors::DenoResult;
use errors::new as deno_error;
use std::process::exit;
use std::path::PathBuf;

const HISTORY_FILE: &str = "history.txt";

pub fn readline(editor: &mut Editor<()>, prompt: &String) -> DenoResult<String> {
  editor
    .readline(prompt)
    .map(|line| {
      editor.add_history_entry(line.as_ref());
      // TODO We'd rather save the history only upon close,
      // but atm we're instantiating a new editor each readline.
      editor.save_history(HISTORY_FILE).unwrap();
      line
    })
    .map_err(|err|
      match err {
        Interrupted => exit(1),
        err => err
      })
    .map_err(|err| deno_error(ErrorKind::Other, err.description().to_string()))
}

pub fn start_repl(name: &String, path: PathBuf) -> Editor<()> {
  let mut editor = Editor::<()>::new();

  // TODO: load history file based on repl name
    let mut history_path: PathBuf = path.clone();
    history_path.push("history");
    history_path.push(name);
    let history_path_str = history_path.to_str().unwrap();

  println!("History file: {}", history_path_str);
  if editor.load_history(history_path_str).is_err() {
    eprintln!("No repl history found, creating new file: {}", history_path_str);
  }
  editor
}
