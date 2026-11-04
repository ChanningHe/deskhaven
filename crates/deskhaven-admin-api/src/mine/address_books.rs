//! `GET /me/address-books`: the signed-in user's personal book and the shared books they own
//! or are granted, each with the rule the user holds (1 read-only, 2 read-write, 3 full
//! control). Read only: entries are edited in the client. The user is the session's; the
//! core service reaches a book only through ownership or a share naming the user or one of
//! their groups, so nobody else's personal book or unshared book appears.
//!
//! The personal book is created on first access, as when the client first asks for it.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::addressbooks::{AddressBookService, Book, Kind, Profile};
use deskhaven_core::pagination::{MAX_PAGE_SIZE, PageRequest};
use deskhaven_core::state::AppState;
use serde::Serialize;
use utoipa::ToSchema;

use crate::AdminApiError;
use crate::address_books::view::rule_number;
use crate::session::ConsoleUser;

/// Pages of shared books read at most (of [`MAX_PAGE_SIZE`] books each).
const MAX_SHARED_PAGES: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BookKindView {
    Personal,
    Shared,
}

/// A book the user can use in the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct MyAddressBook {
    /// The identifier clients use.
    pub guid: String,
    pub kind: BookKindView,
    pub name: String,
    pub note: String,
    /// Username of the owner of a shared book; `null`: managed by the administrators.
    pub owner: Option<String>,
    /// The user's rule: 1 read-only, 2 read-write, 3 full control.
    pub rule: u8,
}

impl MyAddressBook {
    fn personal(book: Book) -> Self {
        Self {
            guid: book.guid,
            kind: BookKindView::Personal,
            name: book.name,
            note: book.note,
            owner: None,
            rule: rule_number(deskhaven_core::addressbooks::Rule::FullControl),
        }
    }

    fn shared(profile: Profile) -> Self {
        let kind = match profile.book.kind {
            Kind::Personal => BookKindView::Personal,
            Kind::Shared => BookKindView::Shared,
        };
        Self {
            guid: profile.book.guid,
            kind,
            name: profile.book.name,
            note: profile.book.note,
            owner: profile.owner,
            rule: rule_number(profile.rule),
        }
    }
}

/// The user's address books: the personal one first, then the shared ones by name.
#[utoipa::path(
    get,
    path = "/me/address-books",
    operation_id = "list_my_address_books",
    tag = "me",
    security(("console_session" = ["self"])),
    responses((status = 200, description = "The books", body = Vec<MyAddressBook>))
)]
pub async fn list(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
) -> Result<Json<Vec<MyAddressBook>>, AdminApiError> {
    let service = AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    );
    let user_id = principal.user_id;
    let personal = service.personal(user_id).await?;
    let mut books = vec![MyAddressBook::personal(personal)];
    for page in 1..=MAX_SHARED_PAGES {
        let request = PageRequest {
            page,
            page_size: MAX_PAGE_SIZE,
        };
        let found = service.shared_profiles(user_id, request).await?;
        let last = found.items.len() < usize::try_from(MAX_PAGE_SIZE).unwrap_or(usize::MAX);
        books.extend(found.items.into_iter().map(MyAddressBook::shared));
        if last {
            break;
        }
    }
    Ok(Json(books))
}
