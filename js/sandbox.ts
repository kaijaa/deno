// Copyright 2018 the Deno authors. All rights reserved. MIT license.
import { libdeno } from "./libdeno";
import { globalEval } from "./global_eval";

const window = globalEval("this");

export interface DenoSandbox {
  // tslint:disable-next-line:no-any
  env: any;
  // tslint:disable-next-line:no-any
  eval: (code: string) => any;
}

function formatFrameAtMessage(frame: { [key: string]: string }) {
  if (frame.functionName) {
    return `    at ${frame.functionName} (${frame.scriptName}:${frame.line}:${
      frame.column
    })`;
  } else if (frame.isEval) {
    return `    at eval (${frame.scriptName}:${frame.line}:${frame.column})`;
  } else {
    return `    at ${frame.scriptName}:${frame.line}:${frame.column}`;
  }
}

// TODO: change type of env
// tslint:disable-next-line:no-any
function parseError(env: any, errMsg: string): Error {
  const errInfo = JSON.parse(errMsg);
  // parse full message (eg. ReferenceError: x is not defined) to get error name and message
  const [errorName, message] = errInfo.message.split(':');
  // try to get actual error or fallback to generic Error
  const errorCtor = env[errorName] || Error;
  const err = new errorCtor();
  err.name = errorName;
  err.message = message.trim();
  const preparedStackFrames = errInfo.frames.map(formatFrameAtMessage).join("\n");
  err.stack = `${errInfo.message}\n${preparedStackFrames}`;
  return err;
}

class DenoSandboxImpl implements DenoSandbox {
  constructor(public env: {}) {}
  eval(code: string) {
    const [result, errMsg] = libdeno.runInContext(this.env, code);
    if (errMsg) {
      let err;
      try {
        err = parseError(this.env, errMsg);
      } catch (e) {
        err = new Error("Unknown sandbox error");
      }
      throw err;
    }
    return result;
  }
}

/** Create a sandboxed context (with a model) to execute code inside.
 *
 *       import * as deno from "deno";
 *       const s = deno.sandbox({a: 1});
 *       s.b = 2;
 *       s.eval("const c = a + b");
 *       console.log(s.c) // prints "3"
 */
export function sandbox(
  model: any // tslint:disable-line:no-any
): DenoSandbox {
  if (typeof model !== "object") {
    throw new Error("Sandbox model has to be an object!");
  }
  // env is the global object of context
  const env = libdeno.makeContext();
  // Copy necessary window properties first
  // To avoid `window.Error !== Error` that causes unexpected behavior
  for (const key of Object.getOwnPropertyNames(window)) {
    try {
      env[key] = model[key];
    } catch (e) {}
  }
  // Then the actual model
  for (const key in model) {
    env[key] = model[key];
  }

  return new DenoSandboxImpl(env);
}
