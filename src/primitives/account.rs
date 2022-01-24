#[derive(Debug, PartialEq)]
pub struct Account {
  pub data: Vec<u8>,
}

impl Account {
  #[cfg(test)]
  pub fn test_new(value: u8) -> Self {
    Self { data: vec![value] }
  }
}
