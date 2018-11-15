// Copyright 2018 the Deno authors. All rights reserved. MIT license.
use getopts::Options;
use getopts::ParsingStyle;
use libc::c_int;
use libdeno;
use std::ffi::CStr;
use std::ffi::CString;
use std::mem;
use std::vec::Vec;

// Creates vector of strings, Vec<String>
#[cfg(test)]
macro_rules! svec {
    ($($x:expr),*) => (vec![$($x.to_string()),*]);
}

#[cfg_attr(feature = "cargo-clippy", allow(stutter))]
#[derive(Debug, PartialEq, Default)]
pub struct DenoFlags {
  pub help: bool,
  pub log_debug: bool,
  pub version: bool,
  pub reload: bool,
  pub recompile: bool,
  pub allow_write: bool,
  pub allow_net: bool,
  pub allow_env: bool,
  pub types: bool,
}

pub fn get_usage(opts: &Options) -> String {
  format!(
    "Usage: deno script.ts {}
Environment variables:
        DENO_DIR        Set deno's base directory.",
    opts.usage("")
  )
}

// Parses flags for deno. This does not do v8_set_flags() - call that separately.
#[cfg_attr(feature = "cargo-clippy", allow(stutter))]
pub fn set_flags(
  args: Vec<String>,
) -> Result<(DenoFlags, Vec<String>, String), String> {
  let args = v8_set_flags(args);

  let mut opts = Options::new();
  // TODO(kevinkassimo): v8_set_flags intercepts '-help' with single '-'
  // Resolve that and then uncomment line below (enabling Go style -long-flag)
  // opts.long_only(true);
  opts.optflag("", "allow-write", "Allow file system write access.");
  opts.optflag("", "allow-net", "Allow network access.");
  opts.optflag("", "allow-env", "Allow environment access.");
  opts.optflag("", "recompile", "Force recompilation of TypeScript code.");
  opts.optflag("h", "help", "Print this message.");
  opts.optflag("D", "log-debug", "Log debug output.");
  opts.optflag("v", "version", "Print the version.");
  opts.optflag("r", "reload", "Reload cached remote resources.");
  opts.optflag("", "v8-options", "Print V8 command line options.");
  opts.optflag("", "types", "Print runtime TypeScript declarations.");

  // set proper parsing style - it allows to pass arguments after script name (and do parsing
  // in userland)
  //     deno --allow-env foo.ts --foo=bar
  // parsing will stop when `foo.ts` is encountered
  opts.parsing_style(ParsingStyle::StopAtFirstFree);

  let mut flags = DenoFlags::default();

  let matches = match opts.parse(&args) {
    Ok(m) => m,
    Err(f) => {
      return Err(f.to_string());
    }
  };

  if matches.opt_present("help") {
    flags.help = true;
  }
  if matches.opt_present("log-debug") {
    flags.log_debug = true;
  }
  if matches.opt_present("version") {
    flags.version = true;
  }
  if matches.opt_present("reload") {
    flags.reload = true;
  }
  if matches.opt_present("recompile") {
    flags.recompile = true;
  }
  if matches.opt_present("allow-write") {
    flags.allow_write = true;
  }
  if matches.opt_present("allow-net") {
    flags.allow_net = true;
  }
  if matches.opt_present("allow-env") {
    flags.allow_env = true;
  }
  if matches.opt_present("types") {
    flags.types = true;
  }

  let rest: Vec<_> = matches.free.to_vec();
  println!("rest args {:?}", rest);
  Ok((flags, rest, get_usage(&opts)))
}

#[test]
fn test_set_flags_1() {
  let (flags, rest, _) = set_flags(svec!["deno", "--version"]).unwrap();
  assert_eq!(rest, svec!["deno"]);
  assert_eq!(
    flags,
    DenoFlags {
      version: true,
      ..DenoFlags::default()
    }
  );
}

#[test]
fn test_set_flags_2() {
  let (flags, rest, _) =
    set_flags(svec!["deno", "-r", "-D", "script.ts"]).unwrap();
  assert_eq!(rest, svec!["deno", "script.ts"]);
  assert_eq!(
    flags,
    DenoFlags {
      log_debug: true,
      reload: true,
      ..DenoFlags::default()
    }
  );
}

#[test]
fn test_set_flags_3() {
  let (flags, rest, _) =
    set_flags(svec!["deno", "-r", "script.ts", "--allow-write"]).unwrap();
  assert_eq!(rest, svec!["deno", "script.ts"]);
  assert_eq!(
    flags,
    DenoFlags {
      reload: true,
      allow_write: true,
      ..DenoFlags::default()
    }
  );
}

#[test]
fn test_set_flags_4() {
  let (flags, rest, _) =
    set_flags(svec!["deno", "-Dr", "script.ts", "--allow-write"]).unwrap();
  assert_eq!(rest, svec!["deno", "script.ts"]);
  assert_eq!(
    flags,
    DenoFlags {
      log_debug: true,
      reload: true,
      allow_write: true,
      ..DenoFlags::default()
    }
  );
}

#[test]
fn test_set_flags_5() {
  let (flags, rest, _) = set_flags(svec!["deno", "--types"]).unwrap();
  assert_eq!(rest, svec!["deno"]);
  assert_eq!(
    flags,
    DenoFlags {
      types: true,
      ..DenoFlags::default()
    }
  )
}

#[test]
fn test_set_bad_flags_1() {
  let err = set_flags(svec!["deno", "--unknown-flag"]).unwrap_err();
  assert_eq!(err, "Unrecognized option: 'unknown-flag'");
}

#[test]
fn test_set_bad_flags_2() {
  // This needs to be changed if -z is added as a flag
  let err = set_flags(svec!["deno", "-z"]).unwrap_err();
  assert_eq!(err, "Unrecognized option: 'z'");
}

// Returns args passed to V8, followed by args passed to JS
fn v8_set_flags_preprocess(args: Vec<String>) -> (Vec<String>, Vec<String>) {
  let (rest, mut v8_args) =
    args.into_iter().partition(|ref a| a.as_str() == "--help");

  // Replace args being sent to V8
  for mut a in &mut v8_args {
    if a == "--v8-options" {
      mem::swap(a, &mut String::from("--help"));
    }
  }
  (v8_args, rest)
}

#[test]
fn test_v8_set_flags_preprocess_1() {
  let js_args = v8_set_flags_preprocess(vec![
    "deno".to_string(),
    "--v8-options".to_string(),
  ]);
  assert_eq!(
    js_args,
    (vec!["deno".to_string(), "--help".to_string()], vec![])
  );
}

#[test]
fn test_v8_set_flags_preprocess_2() {
  let js_args =
    v8_set_flags_preprocess(vec!["deno".to_string(), "--help".to_string()]);
  assert_eq!(
    js_args,
    (vec!["deno".to_string()], vec!["--help".to_string()])
  );
}

// Pass the command line arguments to v8.
// Returns a vector of command line arguments that v8 did not understand.
#[cfg_attr(feature = "cargo-clippy", allow(stutter))]
pub fn v8_set_flags(args: Vec<String>) -> Vec<String> {
  // deno_set_v8_flags(int* argc, char** argv) mutates argc and argv to remove
  // flags that v8 understands.
  // First parse core args, then convert to a vector of C strings.
  let (args, rest) = v8_set_flags_preprocess(args);

  // Make a new array, that can be modified by V8::SetFlagsFromCommandLine(),
  // containing mutable raw pointers to the individual command line args.
  let mut raw_argv = args
    .iter()
    .map(|arg| CString::new(arg.as_str()).unwrap().into_bytes_with_nul())
    .collect::<Vec<_>>();
  let mut c_argv = raw_argv
    .iter_mut()
    .map(|arg| arg.as_mut_ptr() as *mut i8)
    .collect::<Vec<_>>();

  // Store the length of the c_argv array in a local variable. We'll pass
  // a pointer to this local variable to deno_set_v8_flags(), which then
  // updates its value.
  let mut c_argv_len = c_argv.len() as c_int;
  // Let v8 parse the arguments it recognizes and remove them from c_argv.
  unsafe {
    libdeno::deno_set_v8_flags(&mut c_argv_len, c_argv.as_mut_ptr());
  };
  // If c_argv_len was updated we have to change the length of c_argv to match.
  c_argv.truncate(c_argv_len as usize);
  // Copy the modified arguments list into a proper rust vec and return it.
  c_argv
    .iter()
    .map(|ptr| unsafe {
      let cstr = CStr::from_ptr(*ptr as *const i8);
      let slice = cstr.to_str().unwrap();
      slice.to_string()
    }).chain(rest.into_iter())
    .collect()
}
