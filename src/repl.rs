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

pub struct DenoRepl {
    pub name: String,
    pub editor: Editor<()>,
    pub history_file: PathBuf,
}

impl DenoRepl {
    pub fn new(name: &String, path: PathBuf) -> DenoRepl {
        // FIXME: hardcoded path to 'history' directory
        let mut history_path: PathBuf = path.clone();
        history_path.push("history");
        history_path.push(name);

        let mut repl = DenoRepl {
            name: name.clone(),
            editor: Editor::<()>::new(),
            history_file: history_path,
        };

        repl.load_history();
        repl
    }

    fn load_history(&mut self) {
        println!("History file: {}", self.history_file.to_str().unwrap());
        if self.editor.load_history(&self.history_file.to_str().unwrap()).is_err() {}
    }

    fn update_history(&mut self, line: String) {
        self.editor.add_history_entry(line);
        // TODO We'd rather save the history only upon close
        self.editor.save_history(&self.history_file.to_str().unwrap()).unwrap();
    }

    pub fn readline(&mut self, prompt: &String) -> DenoResult<String> {
        self.editor
            .readline(prompt)
            .map(|line| {
                self.update_history(line.clone());
                line
            })
            .map_err(|err|
                match err {
                    Interrupted => exit(1),
                    err => err
                })
            .map_err(|err| deno_error(ErrorKind::Other, err.description().to_string()))
    }
}
