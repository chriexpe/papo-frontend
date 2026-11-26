//! Where each provider's player keeps its cookies and site data: a WebKit network session of
//! its own under Papo's profile (`<profile>/web/<site>/`), never the person's browser profile.
//! One session per site keeps one provider's cookies away from another's player, and lets
//! Clear data remove everything that site's player stored, third parties included
//! (Settings › General › Embedded players). Downloads a page starts are cancelled here.
//!
//! Owns the sessions and the asynchronous fetch and clear; which sites exist and what the
//! settings page shows are the client's.
//!
//! Safety: on the WPE thread. An asynchronous call's data is a box this module made, taken
//! back exactly once by its callback.

#![expect(unsafe_code, reason = "WebKit network sessions and their data manager")]

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use glib_sys::{GError, GFALSE, gpointer};
use gobject_sys::GObject;

use super::glib::{c_string, connect, take_error};
use super::library::{Object, Wpe};
use super::{Events, WebEvent};

/// `WEBKIT_WEBSITE_DATA_ALL`.
const ALL_DATA: u32 = (1 << 12) - 1;
/// `WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE`.
const COOKIES_SQLITE: u32 = 1;

/// The sites' network sessions.
pub(super) struct Sites {
    wpe: &'static Wpe,
    /// `<profile>/web`.
    root: PathBuf,
    sessions: HashMap<String, Object>,
    events: Events,
}

/// What an asynchronous call reports when it ends.
struct Pending {
    wpe: &'static Wpe,
    site: String,
    events: Events,
}

impl Sites {
    pub(super) fn new(wpe: &'static Wpe, root: PathBuf, events: Events) -> Sites {
        Sites {
            wpe,
            root,
            sessions: HashMap::new(),
            events,
        }
    }

    fn directory(&self, site: &str) -> PathBuf {
        self.root.join(site)
    }

    /// `site`'s session, made the first time (its directories too).
    pub(super) fn session(&mut self, site: &str) -> Result<Object, String> {
        if let Some(session) = self.sessions.get(site) {
            return Ok(*session);
        }
        let directory = self.directory(site);
        let data = directory.join("data");
        let cache = directory.join("cache");
        for path in [&data, &cache] {
            std::fs::create_dir_all(path).map_err(|error| error.to_string())?;
        }
        let wpe = self.wpe;
        // SAFETY: WebKit's constructor and the session's own managers; the session is kept
        // until `close`.
        let session = unsafe {
            let session = (wpe.webkit_network_session_new)(
                c_string(&path_text(&data)).as_ptr(),
                c_string(&path_text(&cache)).as_ptr(),
            );
            if session.is_null() {
                return Err("WebKit made no network session".into());
            }
            let cookies = (wpe.webkit_network_session_get_cookie_manager)(session);
            (wpe.webkit_cookie_manager_set_persistent_storage)(
                cookies,
                c_string(&path_text(&directory.join("cookies.sqlite"))).as_ptr(),
                COOKIES_SQLITE,
            );
            let handler: unsafe extern "C" fn(Object, Object, gpointer) = cancel_download;
            connect(
                session,
                c"download-started",
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(Object, Object, gpointer),
                    unsafe extern "C" fn(),
                >(handler)),
                std::ptr::from_ref(wpe).cast_mut().cast(),
            );
            session
        };
        self.sessions.insert(site.to_owned(), session);
        Ok(session)
    }

    /// Finds whether `site` kept anything, and reports it.
    pub(super) fn report(&mut self, site: &str) {
        if !self.directory(site).exists() {
            (self.events)(WebEvent::SiteData {
                site: site.to_owned(),
                has_data: false,
            });
            return;
        }
        let Ok(session) = self.session(site) else {
            return;
        };
        let pending = self.pending(site);
        // SAFETY: the session's data manager; `fetched` takes the box back once.
        unsafe {
            let manager = (self.wpe.webkit_network_session_get_website_data_manager)(session);
            (self.wpe.webkit_website_data_manager_fetch)(
                manager,
                ALL_DATA,
                std::ptr::null_mut(),
                fetched,
                pending,
            );
        }
    }

    /// Removes everything `site` kept, then reports it.
    pub(super) fn clear(&mut self, site: &str) {
        if !self.directory(site).exists() {
            self.report(site);
            return;
        }
        let Ok(session) = self.session(site) else {
            return;
        };
        let pending = self.pending(site);
        // SAFETY: as `report`, with `cleared`.
        unsafe {
            let manager = (self.wpe.webkit_network_session_get_website_data_manager)(session);
            (self.wpe.webkit_website_data_manager_clear)(
                manager,
                ALL_DATA,
                0,
                std::ptr::null_mut(),
                cleared,
                pending,
            );
        }
    }

    fn pending(&self, site: &str) -> gpointer {
        Box::into_raw(Box::new(Pending {
            wpe: self.wpe,
            site: site.to_owned(),
            events: self.events.clone(),
        }))
        .cast()
    }

    /// Lets every session go (the pages using them have closed).
    pub(super) fn close(&mut self) {
        for (_, session) in self.sessions.drain() {
            // SAFETY: the reference `session` took.
            unsafe { gobject_sys::g_object_unref(session.cast()) };
        }
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

unsafe extern "C" fn cancel_download(_session: Object, download: Object, data: gpointer) {
    // SAFETY: the data is the process's symbol table; the download is WebKit's, live for the
    // call.
    unsafe { ((*data.cast::<Wpe>()).webkit_download_cancel)(download) };
}

unsafe extern "C" fn fetched(manager: *mut GObject, result: *mut c_void, data: gpointer) {
    // SAFETY: the box `pending` made; the result belongs to this call.
    let pending = unsafe { Box::from_raw(data.cast::<Pending>()) };
    let wpe = pending.wpe;
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: the manager WebKit passed back; the list and its items are freed here.
    let has_data = unsafe {
        let list =
            (wpe.webkit_website_data_manager_fetch_finish)(manager.cast(), result, &raw mut error);
        let failure = take_error(error);
        if !failure.is_empty() {
            log::warn!("a player site's data wasn't read: {failure}");
        }
        let has_data = !list.is_null();
        glib_sys::g_list_free_full(list, Some(unref_data));
        has_data
    };
    (pending.events)(WebEvent::SiteData {
        site: pending.site,
        has_data,
    });
}

unsafe extern "C" fn unref_data(data: gpointer) {
    // The symbol table is loaded: a fetch only ever starts after it.
    if let Ok(wpe) = super::library::loaded() {
        // SAFETY: a `WebKitWebsiteData` from the fetched list, released once.
        unsafe { (wpe.webkit_website_data_unref)(data) };
    }
}

unsafe extern "C" fn cleared(manager: *mut GObject, result: *mut c_void, data: gpointer) {
    // SAFETY: the box `pending` made; the result belongs to this call.
    let pending = unsafe { Box::from_raw(data.cast::<Pending>()) };
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: the manager WebKit passed back.
    let done = unsafe {
        (pending.wpe.webkit_website_data_manager_clear_finish)(
            manager.cast(),
            result,
            &raw mut error,
        )
    };
    // SAFETY: the error this call returned, freed once.
    let failure = unsafe { take_error(error) };
    if done == GFALSE {
        log::warn!("a player site's data wasn't cleared: {failure}");
    }
    // Ask again rather than assume: what's left is what the page shows.
    let manager = manager.cast::<c_void>();
    let pending = Box::into_raw(pending).cast::<c_void>();
    // SAFETY: as `report`, on the same manager.
    unsafe {
        let wpe = (*pending.cast::<Pending>()).wpe;
        (wpe.webkit_website_data_manager_fetch)(
            manager,
            ALL_DATA,
            std::ptr::null_mut(),
            fetched,
            pending,
        );
    }
}
