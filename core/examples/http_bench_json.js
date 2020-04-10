// This is not a real HTTP server. We read blindly one time into 'requestBuf',
// then write this fixed 'responseBuf'. The point of this benchmark is to

// exercise the event loop in a simple yet semi-realistic way.
const requestBuf = new Uint8Array(64 * 1024);
const responseBuf = new Uint8Array(
  "HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello World\n"
    .split("")
    .map((c) => c.charCodeAt(0))
);

/** Listens on 0.0.0.0:4500, returns rid. */
function listen() {
  return dispatchJson.sendSync("listen", { rid: -1 });
}

/** Accepts a connection, returns rid. */
function accept(rid) {
  return dispatchJson.sendAsync("accept", { rid });
}

/**
 * Reads a packet from the rid, presumably an http request. data is ignored.
 * Returns bytes read.
 */
function read(rid, data) {
  return dispatchJson.sendAsync("read", { rid }, data);
}

/** Writes a fixed HTTP response to the socket rid. Returns bytes written. */
function write(rid, data) {
  return dispatchJson.sendAsync("write", { rid }, data);
}

function close(rid) {
  return dispatchJson.sendSync("close", { rid });
}

async function serve(rid) {
  while (true) {
    const nread = await read(rid, requestBuf);
    if (nread <= 0) {
      break;
    }

    const nwritten = await write(rid, responseBuf);
    if (nwritten < 0) {
      break;
    }
  }
  close(rid);
}

let dispatchJson;

async function main() {
  const errorFactory = (err) => {
    console.error("Op error", err);
  };
  dispatchJson = createDispatchJson(Deno.core, errorFactory);  
  for (const opName in Deno.core.ops()) {
    Deno.core.setAsyncHandler(ops[opName], dispatchJson.handleAsyncMsgFromRust);
  }

  Deno.core.print("http_bench_json.js start\n");

  const listenerRid = listen();
  Deno.core.print(`listening http://127.0.0.1:4544/ rid=${listenerRid}\n`);
  while (true) {
    const rid = await accept(listenerRid);
    // Deno.core.print(`accepted ${rid}`);
    if (rid < 0) {
      Deno.core.print(`accept error ${rid}`);
      return;
    }
    serve(rid);
  }
}

main();
