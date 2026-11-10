//! Tracks promises that were rejected and never given a handler, so an async error is reported
//! instead of vanishing: boa runs every job to completion and returns Ok even when one rejected.

use std::cell::RefCell;

use boa_engine::builtins::promise::{OperationType, Promise};
use boa_engine::context::HostHooks;
use boa_engine::object::builtins::JsPromise;
use boa_engine::object::JsObject;
use boa_engine::{Context, JsValue};

#[derive(Default)]
pub struct Rejections {
    unhandled: RefCell<Vec<JsObject<Promise>>>,
}

impl Rejections {
    /// Why the first promise that was rejected and is still unhandled was rejected.
    pub fn first_reason(&self) -> Option<JsValue> {
        let unhandled = self.unhandled.borrow();
        let promise = JsPromise::from_object(unhandled.first()?.clone().upcast()).ok()?;
        promise.state().as_rejected().cloned()
    }
}

impl HostHooks for Rejections {
    fn promise_rejection_tracker(
        &self,
        promise: &JsObject<Promise>,
        operation: OperationType,
        _context: &mut Context,
    ) {
        let mut unhandled = self.unhandled.borrow_mut();
        match operation {
            OperationType::Reject => unhandled.push(promise.clone()),
            OperationType::Handle => unhandled.retain(|tracked| tracked != promise),
        }
    }
}
