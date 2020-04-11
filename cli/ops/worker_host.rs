// Copyright 2018-2020 the Deno authors. All rights reserved. MIT license.
use super::dispatch_json::{Deserialize, JsonOp, Value};
use crate::fmt_errors::JSError;
use crate::global_state::GlobalState;
use crate::op_error::OpError;
use crate::permissions::DenoPermissions;
use crate::startup_data;
use crate::state::State;
use crate::tokio_util::create_basic_runtime;
use crate::web_worker::WebWorker;
use crate::web_worker::WebWorkerHandle;
use crate::worker::WorkerEvent;
use deno_core::*;
use futures::future::FutureExt;
use std::convert::From;
use std::thread::JoinHandle;

pub fn init(i: &mut Isolate, s: &State) {
  i.register_op("op_create_worker", s.stateful_json_op(op_create_worker));
  i.register_op(
    "op_host_terminate_worker",
    s.stateful_json_op(op_host_terminate_worker),
  );
  i.register_op(
    "op_host_post_message",
    s.stateful_json_op(op_host_post_message),
  );
  i.register_op(
    "op_host_get_message",
    s.stateful_json_op(op_host_get_message),
  );
}

fn create_web_worker(
  global_state: GlobalState,
  permissions: DenoPermissions,
  specifier: ModuleSpecifier,
  use_deno_namespace: bool,
  maybe_name: Option<String>,
) -> Result<WebWorker, ErrBox> {
  let state =
    State::new_for_worker(global_state, Some(permissions), specifier)?;

  let mut worker =
    WebWorker::new(startup_data::deno_isolate_init(), state, maybe_name);
  let script = format!(
    "bootstrapWorkerRuntime(\"{}\", {})",
    worker.name, use_deno_namespace
  );
  worker.execute(&script)?;

  Ok(worker)
}

// TODO(bartlomieju): check if order of actions is aligned to Worker spec
fn run_worker_thread(
  global_state: GlobalState,
  permissions: DenoPermissions,
  specifier: ModuleSpecifier,
  has_source_code: bool,
  source_code: String,
  use_deno_namespace: bool,
  maybe_name: Option<String>,
) -> Result<(JoinHandle<()>, WebWorkerHandle), ErrBox> {
  let (handle_sender, handle_receiver) =
    std::sync::mpsc::sync_channel::<Result<WebWorkerHandle, ErrBox>>(1);

  // FIXME(bartlomieju): make thread name unique
  let builder = std::thread::Builder::new().name("deno-worker".to_string());

  let join_handle = builder.spawn(move || {
    // Any error inside this block is terminal:
    // - JS worker is useless - meaning it throws an exception and can't do anything else,
    //  all action done upon it should be noops
    // - newly spawned thread exits
    let result = create_web_worker(
      global_state,
      permissions,
      specifier.clone(),
      use_deno_namespace,
      maybe_name,
    );

    if let Err(err) = result {
      handle_sender.send(Err(err)).unwrap();
      return;
    }

    let mut worker = result.unwrap();
    let name = worker.name.to_string();
    // Send thread safe handle to newly created worker to host thread
    handle_sender.send(Ok(worker.thread_safe_handle())).unwrap();
    drop(handle_sender);

    // At this point the only method of communication with host
    // is using `worker.internal_channels`.
    //
    // Host can already push messages and interact with worker.
    //
    // Next steps:
    // - create tokio runtime
    // - load provided module or code
    // - start driving worker's event loop

    let mut rt = create_basic_runtime();

    // TODO: run with using select with terminate

    // Execute provided source code immediately
    let result = if has_source_code {
      worker.execute(&source_code)
    } else {
      // TODO(bartlomieju): add "type": "classic", ie. ability to load
      // script instead of module
      let load_future = worker.execute_module(&specifier).boxed_local();

      rt.block_on(load_future)
    };

    if let Err(e) = result {
      let mut sender = worker.internal_channels.sender.clone();
      sender
        .try_send(WorkerEvent::TerminalError(e))
        .expect("Failed to post message to host");

      // Failure to execute script is a terminal error, bye, bye.
      return;
    }

    // TODO(bartlomieju): this thread should return result of event loop
    // that means that we should store JoinHandle to thread to ensure
    // that it actually terminates.
    rt.block_on(worker).expect("Panic in event loop");
    debug!("Worker thread shuts down {}", &name);
  })?;

  let worker_handle = handle_receiver.recv().unwrap()?;
  Ok((join_handle, worker_handle))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWorkerArgs {
  name: Option<String>,
  specifier: String,
  has_source_code: bool,
  source_code: String,
  use_deno_namespace: bool,
}

/// Create worker as the host
fn op_create_worker(
  state: &State,
  args: Value,
  _data: Option<ZeroCopyBuf>,
) -> Result<JsonOp, OpError> {
  let args: CreateWorkerArgs = serde_json::from_value(args)?;

  let specifier = args.specifier.clone();
  let has_source_code = args.has_source_code;
  let source_code = args.source_code.clone();
  let maybe_worker_name = args.name;
  let use_deno_namespace = args.use_deno_namespace;
  let parent_state = state.clone();
  let state = state.borrow();
  let global_state = state.global_state.clone();
  let permissions = state.permissions.clone();
  let referrer = state.main_module.to_string();
  drop(state);

  let module_specifier =
    ModuleSpecifier::resolve_import(&specifier, &referrer)?;

  let (join_handle, worker_handle) = run_worker_thread(
    global_state,
    permissions,
    module_specifier,
    has_source_code,
    source_code,
    use_deno_namespace,
    maybe_worker_name,
  )
  .map_err(|e| OpError::other(e.to_string()))?;
  // At this point all interactions with worker happen using thread
  // safe handler returned from previous function call
  let mut parent_state = parent_state.borrow_mut();
  let worker_id = parent_state.next_worker_id;
  parent_state.next_worker_id += 1;
  parent_state
    .workers
    .insert(worker_id, (join_handle, worker_handle));

  Ok(JsonOp::Sync(json!({ "id": worker_id })))
}

#[derive(Deserialize)]
struct WorkerArgs {
  id: i32,
}

fn op_host_terminate_worker(
  state: &State,
  args: Value,
  _data: Option<ZeroCopyBuf>,
) -> Result<JsonOp, OpError> {
  let args: WorkerArgs = serde_json::from_value(args)?;
  let id = args.id as u32;
  let mut state = state.borrow_mut();
  let (join_handle, worker_handle) =
    state.workers.remove(&id).expect("No worker handle found");
  worker_handle.terminate();
  join_handle.join().expect("Panic in worker thread");
  Ok(JsonOp::Sync(json!({})))
}

fn serialize_worker_event(event: WorkerEvent) -> Value {
  match event {
    WorkerEvent::Message(buf) => json!({ "type": "msg", "data": buf }),
    WorkerEvent::TerminalError(error) => {
      let mut serialized_error = json!({
        "type": "terminalError",
        "error": {
          "message": error.to_string(),
        }
      });

      if let Ok(js_error) = error.downcast::<JSError>() {
        serialized_error = json!({
          "type": "terminalError",
          "error": {
            "message": js_error.message,
            "fileName": js_error.script_resource_name,
            "lineNumber": js_error.line_number,
            "columnNumber": js_error.start_column,
          }
        });
      }

      serialized_error
    }
    WorkerEvent::Error(error) => {
      let mut serialized_error = json!({
        "type": "error",
        "error": {
          "message": error.to_string(),
        }
      });

      if let Ok(js_error) = error.downcast::<JSError>() {
        serialized_error = json!({
          "type": "error",
          "error": {
            "message": js_error.message,
            "fileName": js_error.script_resource_name,
            "lineNumber": js_error.line_number,
            "columnNumber": js_error.start_column,
          }
        });
      }

      serialized_error
    }
  }
}

/// Get message from guest worker as host
fn op_host_get_message(
  state: &State,
  args: Value,
  _data: Option<ZeroCopyBuf>,
) -> Result<JsonOp, OpError> {
  let args: WorkerArgs = serde_json::from_value(args)?;
  let id = args.id as u32;
  let worker_handle = {
    let state_ = state.borrow();
    let (_join_handle, worker_handle) =
      state_.workers.get(&id).expect("No worker handle found");
    worker_handle.clone()
  };
  let state_ = state.clone();
  let op = async move {
    let response = match worker_handle.get_event().await {
      Some(event) => {
        // Terminal error means that worker should be removed from worker table.
        if let WorkerEvent::TerminalError(_) = &event {
          let mut state_ = state_.borrow_mut();
          if let Some((join_handle, mut worker_handle)) =
            state_.workers.remove(&id)
          {
            worker_handle.sender.close_channel();
            join_handle.join().expect("Worker thread panicked");
          }
        }
        serialize_worker_event(event)
      }
      None => {
        // Worker shuts down
        let mut state_ = state_.borrow_mut();
        // Try to remove worker from workers table - NOTE: `Worker.terminate()` might have been called
        // already meaning that we won't find worker in table - in that case ignore.
        if let Some((join_handle, mut worker_handle)) =
          state_.workers.remove(&id)
        {
          worker_handle.sender.close_channel();
          join_handle.join().expect("Worker thread panicked");
        }
        json!({ "type": "close" })
      }
    };
    Ok(response)
  };
  Ok(JsonOp::Async(op.boxed_local()))
}

/// Post message to guest worker as host
fn op_host_post_message(
  state: &State,
  args: Value,
  data: Option<ZeroCopyBuf>,
) -> Result<JsonOp, OpError> {
  let args: WorkerArgs = serde_json::from_value(args)?;
  let id = args.id as u32;
  let msg = Vec::from(data.unwrap().as_ref()).into_boxed_slice();

  debug!("post message to worker {}", id);
  let state = state.borrow();
  let (_, worker_handle) =
    state.workers.get(&id).expect("No worker handle found");
  worker_handle
    .post_message(msg)
    .map_err(|e| OpError::other(e.to_string()))?;
  Ok(JsonOp::Sync(json!({})))
}
