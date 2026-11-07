//! 型システム: 型の表現、宣言の収集、厳密層の型検査、効果/Capability の語彙。

pub mod check;
pub mod effects;
pub mod env;
pub mod ty;

pub use check::{check_action, check_expr_in, check_function, Facts, BUILTINS};
pub use effects::{Capability, Effect, Verb};
pub use env::{collect_env, list_value, resolve_type, ActionSig, AdapterSig, DataInfo, Env, ExternalSig, FnSig};
pub use ty::Type;

#[cfg(test)]
mod tests;
