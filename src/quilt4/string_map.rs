pub trait StringMap<T>: Iterator<Item = String> {
  fn get(&self, key: &str) -> Option<&T>;
  fn insert(&mut self, key: String, value: T);
  fn remove(&mut self, key: &str) -> Option<T>;
}

