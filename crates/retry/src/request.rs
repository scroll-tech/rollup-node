use crate::database::SingleOrMultiple;
use futures::FutureExt;
use sea_orm::{EntityTrait, Select};
use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tower::Service;

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = DatabaseCallProj)]
pub struct DatabaseCall<
    T: EntityTrait,
    Output,
    Map: FnOnce(SingleOrMultiple<T::Model>) -> Output,
    S: Service<DatabaseRequest<T>>,
> {
    #[pin]
    state: DatabaseCallState<T, S>,
    map: Option<Map>,
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
#[warn(clippy::large_enum_variant)]
#[pin_project::pin_project(project = DatabaseCallStateProj)]
pub enum DatabaseCallState<T: EntityTrait, S: Service<DatabaseRequest<T>>> {
    Preparing {
        database_connection: S,
        request: Option<DatabaseRequest<T>>,
    },
    Awaiting {
        #[pin]
        fut: S::Future,
    },
}

#[derive(Debug, Clone)]
pub enum DatabaseRequest<T: EntityTrait> {
    Select { entity: Select<T>, scope: Scope },
}

#[derive(Debug, Clone)]
pub enum Scope {
    All,
    One,
}

impl<T, Output, Map, S> Future for DatabaseCall<T, Output, Map, S>
where
    T: EntityTrait + Unpin,
    Map: FnOnce(SingleOrMultiple<T::Model>) -> Output + Unpin,
    S: Service<DatabaseRequest<T>, Response = SingleOrMultiple<T::Model>> + Unpin,
{
    type Output = Result<Output, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();
        let res = ready!(this.state.poll_unpin(cx));
        Poll::Ready(res.map(|x| this.map.take().expect("future finished")(x)))
    }
}

impl<T, S> Future for DatabaseCallState<T, S>
where
    T: EntityTrait + Unpin,
    S: Service<DatabaseRequest<T>> + Unpin,
{
    type Output = Result<S::Response, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();
        match this {
            DatabaseCallStateProj::Preparing {
                database_connection,
                request,
            } => {
                let fut = database_connection.call(request.take().expect("first call"));
                self.set(Self::Awaiting { fut });
                Poll::Pending
            }
            DatabaseCallStateProj::Awaiting { mut fut } => {
                let res = ready!(fut.poll_unpin(cx));
                Poll::Ready(res)
            }
        }
    }
}
