//! Evaluates a JavaScript snippet in a fresh boa context.
//!
//! `console.log` (and info, warn, error, debug, one shared stream) is shimmed to append to a per-call output buffer instead of
//! doing anything host-visible (boa has no host bindings to begin with).
//! The response combines everything printed with the script's completion
//! value, matching a REPL. Any parse or runtime error is caught and returned
//! as text after whatever was printed before it; nothing here ever panics on user input.

use std::cell::RefCell;
use std::rc::Rc;

use boa_engine::context::time::FixedClock;
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{js_string, Context, JsResult, JsValue, NativeFunction, Source};

thread_local! {
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Runs `source` as a JS script and returns the combined console output and
/// completion value, or an error message on a parse/runtime failure.
///
/// Uses a fixed clock instead of the engine's default `StdClock`: the host
/// ABI forbids importing a clock, and `std::time::Instant`/`SystemTime` both
/// panic at runtime on wasm32-unknown-unknown, which is exactly what the
/// default clock calls as soon as a context is built.
pub fn execute(source: &str) -> Result<String, String> {
    OUTPUT.with(|buf| buf.borrow_mut().clear());

    let mut context = Context::builder()
        .clock(Rc::new(FixedClock::from_millis(0)))
        .build()
        .map_err(|err| err.to_string())?;
    install_console(&mut context).map_err(|err| err.to_string())?;

    let result = context.eval(Source::from_bytes(source)).and_then(|value| context.run_jobs().map(|()| value));
    let printed = OUTPUT.with(|buf| buf.borrow().clone());

    match result {
        Ok(value) => Ok(combine(printed, value, &mut context)),
        Err(err) => Err(with_printed(printed, err.to_string())),
    }
}

fn with_printed(printed: String, error: String) -> String {
    if printed.is_empty() {
        error
    } else {
        format!("{printed}\n{error}")
    }
}

fn combine(printed: String, value: JsValue, context: &mut Context) -> String {
    let mut out = printed;
    if !value.is_undefined() {
        if let Ok(text) = value.to_string(context) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&text.to_std_string_lossy());
        }
    }
    out
}

fn install_console(context: &mut Context) -> JsResult<()> {
    let mut init = ObjectInitializer::new(context);
    for name in [
        js_string!("log"),
        js_string!("info"),
        js_string!("warn"),
        js_string!("error"),
        js_string!("debug"),
    ] {
        init.function(NativeFunction::from_fn_ptr(console_log), name, 0);
    }
    let console = init.build();
    context.register_global_property(js_string!("console"), console, Attribute::all())?;
    Ok(())
}

fn console_log(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut parts = Vec::with_capacity(args.len());
    for arg in args {
        parts.push(match arg.as_string() {
            Some(text) => text.to_std_string_lossy(),
            None => arg.display().to_string(),
        });
    }
    OUTPUT.with(|buf| {
        let mut buf = buf.borrow_mut();
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(&parts.join(" "));
    });
    Ok(JsValue::undefined())
}

#[cfg(test)]
mod tests {
    use super::execute;

    #[test]
    fn output_printed_before_a_throw_is_kept() {
        let err = execute("console.log('step 1'); null.x").unwrap_err();
        assert!(err.starts_with("step 1\n"), "console output lost on throw: {err:?}");
        assert!(err.contains("TypeError"), "the error itself must still be reported: {err:?}");
    }

    #[test]
    fn a_throw_with_nothing_printed_is_just_the_error() {
        let err = execute("throw new Error('x')").unwrap_err();
        assert!(err.starts_with("Error: x"), "{err:?}");
    }

    #[test]
    fn console_error_warn_info_and_debug_print_like_log() {
        for method in ["error", "warn", "info", "debug"] {
            let out = execute(&format!("console.{method}('hello')"));
            assert_eq!(out.as_deref(), Ok("hello"), "console.{method}: {out:?}");
        }
    }

    #[test]
    fn console_log_inspects_objects_instead_of_printing_object_object() {
        let out = execute("console.log({a:1})").unwrap();
        assert!(out.contains("a: 1"), "got {out:?}");
    }

    #[test]
    fn console_log_keeps_strings_unquoted_and_symbols_printable() {
        let out = execute("console.log('a', 1, Symbol('s'))").unwrap();
        assert_eq!(out, "a 1 Symbol(s)");
    }

    #[test]
    fn promise_callbacks_run_before_the_output_is_read() {
        let out = execute("Promise.resolve(7).then(v => console.log(v))").unwrap();
        assert!(out.starts_with("7\n"), "the promise job never ran: {out:?}");
    }

    #[test]
    fn code_after_an_await_runs() {
        let out = execute("(async () => { await null; console.log('after') })()").unwrap();
        assert!(out.starts_with("after"), "the continuation never ran: {out:?}");
    }

    #[test]
    fn output_printed_in_a_job_that_then_throws_is_kept() {
        let err = execute("Promise.resolve().then(() => { console.log('a'); null.x })").unwrap();
        assert!(err.starts_with("a"), "{err:?}");
    }
}
