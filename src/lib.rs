pub mod blocking;
pub mod non_blocking;

pub trait Context: 'static {}
pub trait ThreadSafeContext: Context + Send + Sync {}

#[cfg(test)]
mod tests {}
