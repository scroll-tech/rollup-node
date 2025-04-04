/// The [`DatabaseConnectionProvider`] trait provides a way to get a connection to the database.
/// This is implemented by the [`crate::Database`] and [`crate::DatabaseTransaction`] types.
#[auto_impl::auto_impl(Arc)]
pub trait DatabaseConnectionProvider {
    /// The type of the database connection.
    type Connection: sea_orm::ConnectionTrait + sea_orm::StreamTrait;

    /// Returns a reference to the database connection that implements the `ConnectionTrait` and
    /// `StreamTrait` traits.
    fn get_connection(&self) -> &Self::Connection;
}
