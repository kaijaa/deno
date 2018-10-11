// Copyright 2018 the Deno authors. All rights reserved. MIT license.
extern crate rustyline;
use rustyline::Editor;
use rustyline::error::ReadlineError::Interrupted;

use std::error::Error;
use msg::ErrorKind;

use errors::DenoResult;
use errors::new as deno_error;
use std::process::exit;

const HISTORY_FILE: &str = "history.txt";

pub fn readline(prompt: &String) -> DenoResult<String> {
  // TODO instantiate the editor once only (for the session).
  let mut editor = start_repl();
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

fn start_repl() -> Editor<()> {
  let mut editor = Editor::<()>::new();
  if editor.load_history(HISTORY_FILE).is_err() {
    eprintln!("No repl history found, creating new file: {}", HISTORY_FILE);
  }
  editor
}
