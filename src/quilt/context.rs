// use std::{
//     cell::RefCell,
//     ops::{Deref, DerefMut},
// };

// use super::storage;

pub struct Context {
    // local_storage: Box<dyn storage::LocalStorage>,
    // s3_storage: Box<dyn storage::S3Storage>,
}

// TODO: use some idiomatic way to store the global context stack
// static CONTEXT_STACK: RefCell<Vec<RefCell<Context>>> = RefCell::new(Vec::new());
//
// impl Context {
//     pub fn push(context: Self) {
//         let stack = CONTEXT_STACK.borrow_mut().deref_mut();
//         stack.push(RefCell::new(context));
//     }
//
//     pub fn pop() {
//         let stack = CONTEXT_STACK.borrow_mut().deref_mut();
//         stack.pop();
//     }
//
//     pub fn borrow<'a>() -> &'a RefCell<Self> {
//         let stack = CONTEXT_STACK.borrow().deref();
//         let last = stack.last().unwrap();
//         last
//     }
// }
