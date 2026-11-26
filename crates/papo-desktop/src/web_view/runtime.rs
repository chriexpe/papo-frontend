//! The WebView thread: its GLib main context and loop, the display, the open pages and the
//! sites' sessions. Other threads hand it work through the context (`Thread::run`); it posts
//! what happens through `Events`. Stopping closes every page, lets the sessions go and joins
//! the thread, so the app's exit fence leaves no WebKit work behind.
//!
//! Owns the thread and what lives on it. Safety: the context is made here, kept until the
//! thread has joined, and is the thread-default context of the WebView thread, where every
//! WPE and WebKit call is made.

#![expect(
    unsafe_code,
    reason = "the WebView thread's GLib main context and loop"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;

use glib_sys::GMainLoop;

use super::glib::Context;
use super::library::{Object, Wpe, loaded};
use super::page::{Page, Shared};
use super::sites::Sites;
use super::{Events, PageCall, PageId, PageSpec, WebEvent, classes};

/// The thread, from the other threads' side.
pub(super) struct Thread {
    context: Context,
    handle: Option<JoinHandle<()>>,
}

impl Thread {
    pub(super) fn start(root: PathBuf, render_node: Option<String>, events: Events) -> Thread {
        // SAFETY: a new context, whose reference this keeps until `drop` has joined.
        let context = Context::new(unsafe { glib_sys::g_main_context_new() });
        let handle = std::thread::Builder::new()
            .name("papo-webview".into())
            .spawn(move || run(context, root, render_node.as_deref(), &events))
            .map_err(|error| log::warn!("the WebView thread didn't start: {error}"))
            .ok();
        Thread { context, handle }
    }

    /// Runs `work` with the thread's state, after what was asked before it. Work for a thread
    /// that has stopped (WPE unavailable) is dropped.
    pub(super) fn run(&self, work: impl FnOnce(&mut Runtime) + Send + 'static) {
        self.context.invoke(move || {
            RUNTIME.with_borrow_mut(|runtime| {
                if let Some(runtime) = runtime {
                    work(runtime);
                }
            });
        });
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        self.run(Runtime::stop);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::warn!("the WebView thread panicked");
        }
        // SAFETY: the reference `start` took; the thread is gone, so nothing iterates it.
        unsafe { glib_sys::g_main_context_unref(self.context.raw()) };
    }
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// The WebView thread's state.
pub(super) struct Runtime {
    wpe: &'static Wpe,
    context: Context,
    main_loop: *mut GMainLoop,
    display: Object,
    /// Papo's WebKit context, released here when the thread stops.
    web_context: Object,
    pages: HashMap<PageId, Box<Page>>,
    sites: Sites,
    events: Events,
}

fn run(context: Context, root: PathBuf, render_node: Option<&str>, events: &Events) {
    // SAFETY: this thread takes the context as its default for every source WebKit adds.
    unsafe { glib_sys::g_main_context_push_thread_default(context.raw()) };
    match Runtime::new(context, root, render_node, events) {
        Ok(runtime) => {
            let main_loop = runtime.main_loop;
            RUNTIME.set(Some(runtime));
            events(WebEvent::Ready);
            // SAFETY: the loop made for this context; `stop` quits it.
            unsafe {
                glib_sys::g_main_loop_run(main_loop);
                glib_sys::g_main_loop_unref(main_loop);
            }
            RUNTIME.set(None);
        }
        Err(reason) => {
            log::info!("provider players are off: {reason}");
            events(WebEvent::Unavailable(reason));
        }
    }
    // SAFETY: undoes the push above, on the same thread.
    unsafe { glib_sys::g_main_context_pop_thread_default(context.raw()) };
}

impl Runtime {
    fn new(
        context: Context,
        root: PathBuf,
        render_node: Option<&str>,
        events: &Events,
    ) -> Result<Runtime, String> {
        let wpe = loaded().map_err(|unavailable| unavailable.0)?;
        let types = classes::types(wpe).map_err(|unavailable| unavailable.0)?;
        let display =
            classes::make_display(wpe, types, render_node).map_err(|unavailable| unavailable.0)?;
        // SAFETY: a loop on this thread's context, and a WebKit context this runtime owns.
        let (main_loop, web_context) = unsafe {
            (
                glib_sys::g_main_loop_new(context.raw(), glib_sys::GFALSE),
                (wpe.webkit_web_context_new)(),
            )
        };
        Ok(Runtime {
            wpe,
            context,
            main_loop,
            display,
            web_context,
            pages: HashMap::new(),
            sites: Sites::new(wpe, root, events.clone()),
            events: events.clone(),
        })
    }

    pub(super) fn open(&mut self, id: PageId, spec: PageSpec) {
        self.close(id);
        let session = match self.sites.session(&spec.site) {
            Ok(session) => session,
            Err(error) => {
                log::warn!("a player's site storage didn't open: {error}");
                (self.events)(WebEvent::Page(id, super::PageEvent::Failed));
                return;
            }
        };
        let shared = Shared {
            wpe: self.wpe,
            context: self.context,
            display: self.display,
            web_context: self.web_context,
        };
        match Page::open(shared, session, id, spec, self.events.clone()) {
            Ok(page) => {
                self.pages.insert(id, page);
            }
            Err(error) => {
                log::warn!("a provider page didn't open: {error}");
                (self.events)(WebEvent::Page(id, super::PageEvent::Failed));
            }
        }
    }

    pub(super) fn call(&mut self, id: PageId, call: PageCall) {
        let Some(page) = self.pages.get(&id) else {
            return;
        };
        match call {
            PageCall::Resize { size_pt, scale } => page.resize(size_pt, scale),
            PageCall::Input(input) => page.input(input),
            PageCall::Focus(focused) => page.focus(focused),
            PageCall::Mute(muted) => page.mute(muted),
        }
    }

    pub(super) fn close(&mut self, id: PageId) {
        if let Some(page) = self.pages.remove(&id) {
            page.close();
        }
    }

    pub(super) fn report_site(&mut self, site: &str) {
        self.sites.report(site);
    }

    pub(super) fn clear_site(&mut self, site: &str) {
        self.sites.clear(site);
    }

    fn stop(&mut self) {
        for (_, page) in self.pages.drain() {
            page.close();
        }
        self.sites.close();
        // SAFETY: the pages using the context are closed; the loop `run` is running.
        unsafe {
            gobject_sys::g_object_unref(self.web_context.cast());
            glib_sys::g_main_loop_quit(self.main_loop);
        }
    }
}
