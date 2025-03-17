/// The [`DatabaseConnectionProvider`] trait provides a way to get a connection to the database.
/// This is implemented by the [`crate::Database`] and [`crate::DatabaseTransaction`] types.
pub trait DatabaseConnectionProvider {
    /// Returns a reference to the database connection that implements the `ConnectionTrait` and
    /// `StreamTrait` traits.
    fn get_connection(&self) -> &(impl sea_orm::ConnectionTrait + sea_orm::StreamTrait);
}
