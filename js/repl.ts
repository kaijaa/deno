// Copyright 2018 the Deno authors. All rights reserved. MIT license.
import * as msg from "gen/msg_generated";
import { flatbuffers } from "flatbuffers";
import { assert } from "./util";
import * as deno from "./deno";
import * as dispatch from "./dispatch";
import { exit } from "./os";
import { window } from "./globals";

// @internal
export async function readline(prompt: string): Promise<string> {
  return res(await dispatch.sendAsync(...req(prompt)));
}

function req(
  prompt: string
): [flatbuffers.Builder, msg.Any, flatbuffers.Offset] {
  const builder = new flatbuffers.Builder();
  const prompt_ = builder.createString(prompt);
  msg.Repl.startRepl(builder);
  msg.Repl.addPrompt(builder, prompt_);
  const inner = msg.Repl.endRepl(builder);
  return [builder, msg.Any.Repl, inner];
}

function res(baseRes: null | msg.Base): string {
  assert(baseRes != null);
  assert(msg.Any.ReplRes === baseRes!.innerType());
  const inner = new msg.ReplRes();
  assert(baseRes!.inner(inner) != null);
  const line = inner.line();
  assert(line !== null);
  return line || "";
}

// @internal
export async function replLoop(): Promise<void> {
  window.deno = deno;  // FIXME use a new scope (rather than window).
  let line = "";
  while(true){
    try {
      line = await readline(">> ");
      line = line.trim();
    } catch(err) {
      if (err.message === "EOF") { break; }
      console.error(err);
      exit(1);
    }
    if (!line) { continue; }
    if (line === ".exit") { break; }
    try {
      const result = eval.call(window, line);  // FIXME use a new scope.
      console.log(result);
    } catch (err) {
      if (err instanceof Error) {
        console.error(`${err.constructor.name}: ${err.message}`);
      } else {
        console.error("Thrown:", err);
      }
    }
  }
}
