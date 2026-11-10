//! Evaluates a JavaScript snippet in a fresh boa context.
//!
//! `console.log` (and info, warn, error, debug, one shared stream) is shimmed to append to a per-call output buffer instead of
//! doing anything host-visible (boa has no host bindings to begin with).
//! The response combines everything printed with the script's completion
//! value, matching a REPL: a promise stands for what it settled to, and one that was
//! rejected with no handler is an error. Any parse or runtime error is caught and returned
//! as text after whatever was printed before it; nothing here ever panics on user input.

use std::cell::RefCell;
use std::rc::Rc;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::context::time::FixedClock;
use boa_engine::object::builtins::JsPromise;
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{js_string, Context, JsError, JsResult, JsValue, NativeFunction, Source};

use crate::output::Printed;
use crate::rejections::Rejections;

thread_local! {
    static OUTPUT: RefCell<Printed> = const { RefCell::new(Printed::new()) };
}

/// Runs `source` as a JS script and returns the combined console output and
/// completion value, or an error message on a parse/runtime failure.
///
/// Uses a fixed clock instead of the engine's default `StdClock`: the host
/// ABI forbids importing a clock, and `std::time::Instant`/`SystemTime` both
/// panic at runtime on wasm32-unknown-unknown, which is exactly what the
/// default clock calls as soon as a context is built.
pub fn execute(source: &str) -> Result<String, String> {
    OUTPUT.with(|buf| buf.replace(Printed::new()));

    let rejections = Rc::new(Rejections::default());
    let mut context = Context::builder()
        .clock(Rc::new(FixedClock::from_millis(0)))
        .host_hooks(rejections.clone())
        .build()
        .map_err(|err| err.to_string())?;
    install_console(&mut context).map_err(|err| err.to_string())?;

    let result = context.eval(Source::from_bytes(source)).and_then(|value| context.run_jobs().map(|()| value));
    let printed = OUTPUT.with(RefCell::take);

    let outcome = result.map_err(|err| err.to_string()).and_then(|value| settle(value, &rejections, &mut context));
    match outcome {
        Ok(value) => Ok(printed.finish(value.as_deref())),
        Err(error) => Err(printed.finish(Some(&error))),
    }
}

/// The text of the script's value, or the error a rejection nobody handled left behind.
fn settle(value: JsValue, rejections: &Rejections, context: &mut Context) -> Result<Option<String>, String> {
    if let Some(reason) = rejections.first_reason() {
        return Err(JsError::from_opaque(reason).to_string());
    }
    let value = match promise_state(&value) {
        Some(PromiseState::Fulfilled(settled)) => settled,
        Some(_) => return Ok(Some(value.display().to_string())),
        None => value,
    };
    if value.is_undefined() {
        return Ok(None);
    }
    Ok(value.to_string(context).ok().map(|text| text.to_std_string_lossy()))
}

fn promise_state(value: &JsValue) -> Option<PromiseState> {
    let promise = JsPromise::from_object(value.as_object()?.clone()).ok()?;
    Some(promise.state())
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

fn console_log(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> JsResult<JsValue> {
    let mut parts = Vec::with_capacity(args.len());
    for arg in args {
        parts.push(match arg.as_string() {
            Some(text) => text.to_std_string_lossy(),
            None => arg.display().to_string(),
        });
    }
    OUTPUT.with(|buf| buf.borrow_mut().push_line(&parts.join(" ")));
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
        assert_eq!(out, "7");
    }

    #[test]
    fn code_after_an_await_runs() {
        let out = execute("(async () => { await null; console.log('after') })()").unwrap();
        assert_eq!(out, "after");
    }

    #[test]
    fn a_promise_is_shown_as_what_it_resolved_to() {
        assert_eq!(execute("Promise.resolve(7)").unwrap(), "7");
        assert_eq!(execute("(async () => 5)()").unwrap(), "5");
        assert_eq!(execute("console.log('a'); (async () => 5)()").unwrap(), "a\n5");
        assert_eq!(execute("Promise.resolve()").unwrap(), "");
    }

    #[test]
    fn a_promise_that_never_settles_says_so() {
        assert_eq!(execute("new Promise(() => {})").unwrap(), "Promise { <pending> }");
    }

    #[test]
    fn an_async_function_that_throws_is_an_error() {
        let err = execute("async function main() { throw new Error('boom') } main()").unwrap_err();
        assert!(err.starts_with("Error: boom"), "{err:?}");
        assert!(!err.contains("[object Promise]"), "{err:?}");
    }

    #[test]
    fn a_rejected_promise_is_an_error_after_what_was_printed() {
        let bare = execute("Promise.reject(new Error('x'))").unwrap_err();
        assert!(bare.starts_with("Error: x") && !bare.contains("[object Promise]"), "{bare:?}");
        let after_print = execute("console.log('a'); Promise.reject(new Error('x'))").unwrap_err();
        assert!(after_print.starts_with("a\nError: x"), "{after_print:?}");
    }

    #[test]
    fn a_rejection_nobody_awaits_is_an_error_even_when_it_is_not_the_value() {
        let err = execute("Promise.reject(new Error('lost')); 1 + 1").unwrap_err();
        assert!(err.starts_with("Error: lost"), "{err:?}");
    }

    #[test]
    fn a_rejection_with_a_plain_value_is_an_error() {
        assert_eq!(execute("Promise.reject(42)").unwrap_err(), "42");
    }

    #[test]
    fn a_rejection_that_is_caught_is_not_an_error() {
        let out = execute("Promise.reject(new Error('x')).catch(e => console.log('caught', e.message))").unwrap();
        assert_eq!(out, "caught x");
    }

    #[test]
    fn a_rejection_caught_in_a_later_job_is_not_an_error() {
        let out = execute("const p = Promise.reject(new Error('x')); Promise.resolve().then(() => p.catch(e => console.log('late', e.message)))").unwrap();
        assert_eq!(out, "late x");
    }

    #[test]
    fn a_try_catch_around_an_await_catches_the_rejection() {
        let out = execute("(async () => { try { await Promise.reject(new Error('x')) } catch (e) { console.log('got', e.message) } })()").unwrap();
        assert_eq!(out, "got x");
    }

    #[test]
    fn output_printed_in_a_job_that_then_throws_is_kept() {
        let err = execute("Promise.resolve().then(() => { console.log('a'); null.x })").unwrap_err();
        assert!(err.starts_with("a\nTypeError"), "{err:?}");
    }

    #[test]
    fn a_script_that_prints_without_end_is_cut_short_with_a_notice() {
        let out = execute("for (let i = 0; i < 5000; i++) console.log('line ' + i + ' ' + 'x'.repeat(100))").unwrap();
        assert!(out.starts_with("line 0 xxx"), "the start is kept: {:?}", &out[..40]);
        assert!(out.ends_with("\n... output truncated"), "{:?}", &out[out.len() - 40..]);
        assert!(out.len() <= 64 * 1024 + 32, "{} bytes", out.len());
    }

    #[test]
    fn one_enormous_line_is_cut_on_a_character_boundary() {
        let out = execute("console.log('\u{e9}'.repeat(100000))").unwrap();
        let body = out.strip_suffix("\n... output truncated").expect("a notice");
        assert!(body.len() <= 64 * 1024 && body.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn an_error_is_still_reported_behind_output_that_was_cut() {
        let err = execute("for (let i = 0; i < 5000; i++) console.log('x'.repeat(100)); throw new Error('late')").unwrap_err();
        assert!(err.contains("\n... output truncated\nError: late"), "{:?}", &err[err.len() - 100..]);
    }

    #[test]
    fn a_huge_completion_value_is_cut_too() {
        let out = execute("'x'.repeat(200000)").unwrap();
        assert!(out.ends_with("x\n... output truncated") && out.len() <= 64 * 1024 + 32, "{} bytes", out.len());
    }

    #[test]
    fn output_just_under_the_cap_is_not_touched() {
        let out = execute("console.log('x'.repeat(60000))").unwrap();
        assert_eq!(out.len(), 60000);
    }
}
