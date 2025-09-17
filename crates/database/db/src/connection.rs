/// The [`DatabaseConnectionProvider`] trait provides a way to get a connection to the database.
#[auto_impl::auto_impl(Arc)]
pub trait DatabaseConnectionProvider {
    /// The type of the database connection.
    type Connection: sea_orm::ConnectionTrait + sea_orm::StreamTrait;

    /// Returns a reference to the database connection that implements the `ConnectionTrait` and
    /// `StreamTrait` traits.
    fn get_connection(&self) -> &Self::Connection;
}

/// A trait for providing read-only access to the database.
pub trait ReadConnectionProvider: DatabaseConnectionProvider {}

/// A trait for providing read and write access to the database.
pub trait WriteConnectionProvider: ReadConnectionProvider {}
