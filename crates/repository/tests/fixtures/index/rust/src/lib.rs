pub mod math;
use crate::math::base;

pub fn app() -> i32 { base(2) }
#[test]
fn test_app() { assert_eq!(app(), 3); }
