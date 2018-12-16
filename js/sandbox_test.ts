// Copyright 2018 the Deno authors. All rights reserved. MIT license.
import { test, assert, assertEqual } from "./test_util.ts";
import * as deno from "deno";

test(function sandboxEval() {
  const model = { a: 1, b: 2 };
  const s = deno.sandbox(model);
  assertEqual(s.eval("a + b"), 3);
});

test(function sandboxLexicalScope() {
  const model = { a: 1, b: 2 };
  const s = deno.sandbox(model);
  s.eval("const c = 10");
  assertEqual(s.eval("c"), 10);
});

test(function sandboxError() {
  const model = { a: 1 };
  const s = deno.sandbox(model);
  try {
    s.eval("not_a_variable");
  } catch (err) {
    assert(!!err);
    // TODO: somehow this is not ReferenceError instance but generic Error, that's why name is checked
    assert(err.name, "ReferenceError");
    assertEqual(err.message, "not_a_variable is not defined");
    return;
  }
  assert(false, "Error should be caught.");
});

test(function sandboxSetEnv() {
  const model = { a: 1, b: 2 };
  const s = deno.sandbox(model);
  s.env.c = 3;
  assertEqual(s.eval("c"), 3);
});

test(function sandboxGetEnv() {
  const model = { a: 1, b: 2, c: 0 };
  const s = deno.sandbox(model);
  assertEqual(s.env.c, 0);
  s.eval("c = a + b");
  assertEqual(s.env.c, 3);
});
