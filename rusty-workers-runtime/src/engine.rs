use rusty_v8 as v8;
use crate::error::*;
use rusty_workers::types::*;
use std::convert::TryFrom;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Alias for callback function types.
pub trait Callback = Fn(&mut v8::HandleScope, v8::FunctionCallbackArguments, v8::ReturnValue) + Copy + Sized;

/// Shared container for `TerminationReason`.
#[derive(Clone)]
pub struct TerminationReasonBox(pub Arc<Mutex<TerminationReason>>);

/// Reason of execution termination.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TerminationReason {
    /// Terminated for unknown reason.
    Unknown,

    /// Time limit exceeded.
    TimeLimit,

    /// Memory limit exceeded.
    MemoryLimit,
}

/// Utility trait for converting a `Global` into a `Local`.
/// 
/// Directly calling `Local::new(scope, do_something_with(scope))` has lifetime issues so added
/// this for convenience.
pub trait IntoLocal {
    type Target;

    /// Converts `self` into a `Local`.
    fn into_local<'s>(self, scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, Self::Target>;
}

impl<T> IntoLocal for v8::Global<T> {
    type Target = T;
    fn into_local<'s>(self, scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, Self::Target> {
        v8::Local::new(scope, self)
    }
}

/// Utility trait for checking the reason of a V8 error.
pub trait CheckedResult {
    type Target;
    fn check<'s>(self, scope: &mut v8::HandleScope<'s>) -> GenericResult<Self::Target>;
}

impl<T> CheckedResult for Option<T> {
    type Target = T;
    fn check<'s>(self, scope: &mut v8::HandleScope<'s>) -> GenericResult<Self::Target> {
        self.ok_or_else(|| check_exception(scope))
    }
}

/// Converts a `Result` from a callback function into a JavaScript exception.
pub fn wrap_callback<'s, F: FnOnce(&mut v8::HandleScope<'s>) -> JsResult<()>>(scope: &mut v8::HandleScope<'s>, f: F) {
    match f(scope) {
        Ok(()) => {}
        Err(e) => {
            let exception = e.build(scope);
            scope.throw_exception(exception);
        }
    }
}

pub fn make_function<'s, C: Callback>(scope: &mut v8::HandleScope<'s>, native: C) -> GenericResult<v8::Local<'s, v8::Function>> {
    Ok(v8::Function::new(scope, native).check(scope)?)
}

pub fn make_object<'s>(scope: &mut v8::HandleScope<'s>) -> GenericResult<v8::Local<'s, v8::Object>> {
    Ok(v8::Object::new(scope))
}

pub fn make_string<'s>(scope: &mut v8::HandleScope<'s>, s: &str) -> GenericResult<v8::Local<'s, v8::String>> {
    Ok(v8::String::new(scope, s).check(scope)?)
}

pub fn is_instance_of<'s, 'i, 'j>(scope: &mut v8::HandleScope<'s>, constructor: v8::Local<'i, v8::Function>, object: v8::Local<'j, v8::Object>) -> GenericResult<bool> {
    let key = make_string(scope, "prototype")?;
    let expected_proto = constructor.get(scope, key.into()).check(scope)?;
    let actual_proto = object.get_prototype(scope).check(scope)?;
    Ok(expected_proto.same_value(actual_proto))
}

pub fn make_persistent_class<'s, C: Callback>(scope: &mut v8::HandleScope<'s>, constructor: C, elements: BTreeMap<String, v8::Local<'s, v8::Value>>) -> GenericResult<v8::Global<v8::Function>> {
    let constructor = make_function(scope, constructor)?;
    let key = make_string(scope, "prototype")?;
    let proto = constructor.get(scope, key.into()).check(scope)?;
    let proto = v8::Local::<'_, v8::Object>::try_from(proto).map_err(|_| GenericError::Other("make_persistent_class: cannot convert prototype".into()))?;
    for (k, v) in elements {
        let k = v8::String::new(scope, k.as_str()).check(scope)?;
        proto.set(scope, k.into(), v);
    }
    Ok(v8::Global::new(scope, constructor))
}

pub fn add_props_to_object<'s>(scope: &mut v8::HandleScope<'s>, obj: &v8::Local<'s, v8::Object>, elements: BTreeMap<String, v8::Local<'s, v8::Value>>) -> GenericResult<()> {
    for (k, v) in elements {
        let k = v8::String::new(scope, k.as_str()).check(scope)?;
        obj.set(scope, k.into(), v);
    }
    Ok(())
}

fn check_exception(isolate: &mut v8::Isolate) -> GenericError {
    let termination_reason = isolate.get_slot_mut::<TerminationReasonBox>().unwrap().0.lock().unwrap().clone();
    match termination_reason {
        TerminationReason::Unknown => GenericError::ScriptThrowsException,
        TerminationReason::TimeLimit => GenericError::TimeLimitExceeded,
        TerminationReason::MemoryLimit => GenericError::MemoryLimitExceeded,
    }
}