
use async_trait::async_trait;
#[async_trait]
pub trait StringMap<'a, T> {
  async fn get(&self, key: &str) -> Option<&T>;
  async fn insert(&mut self, key: &str, value: &T);
  async fn iter(&self) -> StringIterator;
}

pub struct StringIterator {
  pub items: Vec<String>,
  index: usize,
}

impl StringIterator {
  pub fn new(items: Vec<String>) -> Self {
    StringIterator {
      items,
      index: 0,
    }
  }
}

impl Iterator for StringIterator {
  type Item = String;

  fn next(&mut self) -> Option<Self::Item> {
    if self.index < self.items.len() {
      let item = self.items[self.index].clone();
      self.index += 1;
      Some(item)
    } else {
      None
    }
  }
}

// TODO: add default implementations for "values" and "items" iterators
