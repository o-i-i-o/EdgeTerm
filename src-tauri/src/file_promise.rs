//! Dragging a remote entry out of the window without downloading it first.
//!
//! macOS lets a drag carry a *promise* of a file (`NSFilePromiseProvider`):
//! the pasteboard holds a name and a type, and a drop target that accepts the
//! promise names the path it wants the file written to. That is the only
//! moment the download runs, and it goes straight to that path — a drag that
//! is cancelled, or dropped back inside the window, never touches the server.
//! Windows and Linux have no equivalent in the `drag` crate; there a remote
//! entry is still staged before the drag (see `commands::start_file_drag`).
//!
//! The download itself stays in the Filer: a `Write` event names the
//! destination, the frontend runs the transfer it already knows how to show
//! and cancel, and `finish` hands the outcome to the receiver waiting on it.

use tauri::ipc::Channel;

use crate::error::Result;

/// What a promise drag reports back: `Write` when a receiver asks for the
/// file (once per drop in practice, and possibly after `Ended`), `Ended` once
/// when the pointer is released, `Failed` instead of both when the drag could
/// not be started.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PromisedDragEvent {
    /// A drop target took the promise and wants the entry at `destination`,
    /// a full local path that already carries the entry's name. `token`
    /// goes back through `finish` once the file is there, or is not coming.
    Write {
        token: String,
        destination: String,
    },
    /// The drag ended; `dropped` is false when it was cancelled or let go
    /// somewhere that took nothing (inside this window, say).
    Ended {
        dropped: bool,
    },
    Failed {
        error: String,
    },
}

/// Starts a drag out of `window` that promises an entry called `name`. The
/// pointer must still be down, as with `commands::start_file_drag`.
pub fn start(
    window: &tauri::WebviewWindow,
    name: String,
    is_dir: bool,
    icon: &'static [u8],
    on_event: Channel<PromisedDragEvent>,
) -> Result<()> {
    imp::start(window, name, is_dir, icon, on_event)
}

/// Settles the write a `Write` event asked for: the receiver gets its file
/// (`error` is `None` and the file is at the destination it named), or is
/// told why it is not coming.
pub fn finish(window: &tauri::WebviewWindow, token: String, error: Option<String>) -> Result<()> {
    imp::finish(window, token, error)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{
        AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    };
    use objc2_app_kit::{
        NSApp, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
        NSDraggingSource, NSEvent, NSEventModifierFlags, NSEventType, NSFilePromiseProvider,
        NSFilePromiseProviderDelegate, NSImage, NSView,
    };
    use objc2_foundation::{
        NSArray, NSData, NSDictionary, NSError, NSLocalizedDescriptionKey, NSOperationQueue,
        NSPoint, NSRect, NSString, NSURL,
    };
    use tauri::ipc::Channel;

    use super::PromisedDragEvent;
    use crate::error::{Result, err};

    /// The block a receiver hands over with the destination; calling it is
    /// what tells the receiver the file is there (or not).
    type Completion = RcBlock<dyn Fn(*mut NSError)>;

    /// How long a requested write may stay unanswered before the receiver is
    /// told it is not coming.
    ///
    /// This is the page dying, not a slow link: the clock starts when the
    /// receiver asks and stops when `finish` arrives, which for a large file
    /// over a slow link is minutes later. An hour is longer than a transfer
    /// this application can be expected to run, so the deadline only fires
    /// where nothing will ever report. It is swept at the entry points below,
    /// so a page that dies with no further drag leaves the receiver waiting
    /// until the next one.
    #[cfg(not(test))]
    const WRITE_DEADLINE: Duration = Duration::from_secs(60 * 60);
    /// Short enough for a test to wait it out.
    #[cfg(test)]
    const WRITE_DEADLINE: Duration = Duration::from_millis(50);

    /// Delegates kept alive at once. A provider only holds its delegate
    /// weakly, so every drag needs one here until the receiver asks for the
    /// file; keeping only the newest dropped the request of a drag that was
    /// still in flight, and keeping every one of them would leak a class
    /// instance per drag.
    const LIVE_DELEGATES: usize = 8;

    // Everything here runs on the main thread: the drag is started there, and
    // the promise delegate asks for the main queue, so the pending writes need
    // no lock.
    thread_local! {
        /// Writes the Filer is still running, by the token it was given, with
        /// the moment the receiver asked for the file.
        static PENDING: RefCell<HashMap<String, (Instant, Completion)>> = RefCell::new(HashMap::new());
        /// The most recent delegates. A receiver that asks for the file later
        /// than the `LIVE_DELEGATES` drags after its own finds no delegate and
        /// gets nothing, which beats a dangling one.
        static LIVE: RefCell<Vec<Retained<PromiseDelegate>>> = const { RefCell::new(Vec::new()) };
    }

    /// Fails the writes whose receiver has waited past `WRITE_DEADLINE`.
    /// Called from the main-thread entry points, all of which run while a
    /// drag is being set up or answered; sweeping needs nothing else. Nothing
    /// sweeps on a timer, so a page that dies mid-download holds its receiver
    /// until the next drag — which is why the deadline has to be generous.
    fn sweep_expired() {
        let expired: Vec<Completion> = PENDING.with(|pending| {
            let mut pending = pending.borrow_mut();
            let stale: Vec<String> = pending
                .iter()
                .filter(|(_, (asked, _))| asked.elapsed() >= WRITE_DEADLINE)
                .map(|(token, _)| token.clone())
                .collect();
            stale
                .into_iter()
                .filter_map(|token| pending.remove(&token).map(|(_, completion)| completion))
                .collect()
        });
        for completion in expired {
            fail(&completion, "the download did not start in time");
        }
    }

    struct DelegateIvars {
        name: String,
        on_event: Channel<PromisedDragEvent>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "EdgeTermFilePromiseDelegate"]
        #[ivars = DelegateIvars]
        struct PromiseDelegate;

        unsafe impl NSObjectProtocol for PromiseDelegate {}

        unsafe impl NSFilePromiseProviderDelegate for PromiseDelegate {
            #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
            fn file_name(
                &self,
                _provider: &NSFilePromiseProvider,
                _file_type: &NSString,
            ) -> Retained<NSString> {
                NSString::from_str(&self.ivars().name)
            }

            #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
            fn write_promise(
                &self,
                _provider: &NSFilePromiseProvider,
                url: &NSURL,
                completion: &DynBlock<dyn Fn(*mut NSError)>,
            ) {
                sweep_expired();
                let completion = completion.copy();
                let Some(destination) = url.path().map(|path| path.to_string()) else {
                    fail(&completion, "the drop target named no file path");
                    return;
                };
                let token = uuid::Uuid::new_v4().to_string();
                PENDING.with(|pending| {
                    pending
                        .borrow_mut()
                        .insert(token.clone(), (Instant::now(), completion))
                });
                let _ = self
                    .ivars()
                    .on_event
                    .send(PromisedDragEvent::Write { token, destination });
            }

            #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
            fn operation_queue(
                &self,
                _provider: &NSFilePromiseProvider,
            ) -> Retained<NSOperationQueue> {
                NSOperationQueue::mainQueue()
            }
        }
    );

    impl PromiseDelegate {
        fn new(
            name: String,
            on_event: Channel<PromisedDragEvent>,
            mtm: MainThreadMarker,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(DelegateIvars { name, on_event });
            unsafe { msg_send![super(this), init] }
        }
    }

    struct SourceIvars {
        on_event: Channel<PromisedDragEvent>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "EdgeTermPromiseDragSource"]
        #[ivars = SourceIvars]
        struct PromiseDragSource;

        unsafe impl NSObjectProtocol for PromiseDragSource {}

        unsafe impl NSDraggingSource for PromiseDragSource {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn source_operation_mask(
                &self,
                _session: &NSDraggingSession,
                context: NSDraggingContext,
            ) -> NSDragOperation {
                // Inside EdgeTerm the promise has nowhere to go: the terminal
                // and the Filer take paths, not promises. Refusing up front
                // shows the "not allowed" cursor there instead of a drop that
                // looks accepted and does nothing.
                if context == NSDraggingContext::WithinApplication {
                    NSDragOperation::None
                } else {
                    NSDragOperation::Copy
                }
            }

            #[unsafe(method(draggingSession:endedAtPoint:operation:))]
            fn ended(
                &self,
                _session: &NSDraggingSession,
                _point: NSPoint,
                operation: NSDragOperation,
            ) {
                let _ = self.ivars().on_event.send(PromisedDragEvent::Ended {
                    dropped: operation != NSDragOperation::None,
                });
            }
        }
    );

    impl PromiseDragSource {
        fn new(on_event: Channel<PromisedDragEvent>, mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(SourceIvars { on_event });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Tells the receiver its file is not coming, and why.
    fn fail(completion: &Completion, message: &str) {
        let description = NSString::from_str(message);
        let user_info = NSDictionary::from_slices(
            &[unsafe { NSLocalizedDescriptionKey }],
            &[&*description as &AnyObject],
        );
        let error = unsafe {
            NSError::errorWithDomain_code_userInfo(
                &NSString::from_str("EdgeTerm"),
                1,
                Some(&user_info),
            )
        };
        completion.call((Retained::as_ptr(&error) as *mut NSError,));
    }

    pub fn start(
        window: &tauri::WebviewWindow,
        name: String,
        is_dir: bool,
        icon: &'static [u8],
        on_event: Channel<PromisedDragEvent>,
    ) -> Result<()> {
        let target = window.clone();
        // AppKit only starts a drag from the main thread, and a command is not
        // promised to run on it. The outcome comes back through `on_event`.
        window
            .run_on_main_thread(move || {
                sweep_expired();
                let failed = |error: String| {
                    let _ = on_event.send(PromisedDragEvent::Failed { error });
                };
                let Some(mtm) = MainThreadMarker::new() else {
                    return failed("not on the main thread".into());
                };
                let view = match target.ns_view() {
                    // SAFETY: Tauri hands out the NSView of a live webview,
                    // and this closure runs on the thread that owns it.
                    Ok(view) => unsafe { &*(view as *const NSView) },
                    Err(error) => return failed(error.to_string()),
                };
                let (Some(ns_window), Some(content_view)) =
                    (view.window(), view.window().and_then(|window| window.contentView()))
                else {
                    return failed("the window has gone".into());
                };

                let position = ns_window.mouseLocationOutsideOfEventStream();
                let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(icon))
                else {
                    return failed("could not decode the drag icon".into());
                };
                let size = image.size();
                let frame = NSRect::new(
                    NSPoint::new(position.x - size.width / 2.0, position.y - size.height / 2.0),
                    size,
                );

                // The type only tells the receiver what kind of thing to expect;
                // the name carries the extension. A folder promise is what makes
                // Finder hand over a directory path to fill.
                let file_type = NSString::from_str(if is_dir { "public.folder" } else { "public.data" });
                let delegate = PromiseDelegate::new(name, on_event.clone(), mtm);
                let provider = NSFilePromiseProvider::initWithFileType_delegate(
                    NSFilePromiseProvider::alloc(),
                    &file_type,
                    ProtocolObject::from_ref(&*delegate),
                );
                let item = NSDraggingItem::initWithPasteboardWriter(
                    NSDraggingItem::alloc(),
                    ProtocolObject::from_ref(&*provider),
                );
                // SAFETY: the frame is in the window's coordinate space and
                // the image outlives the call.
                unsafe { item.setDraggingFrame_contents(frame, Some(&image)) };

                let timestamp = NSApp(mtm)
                    .currentEvent()
                    .map(|event| event.timestamp())
                    .unwrap_or(0.0);
                let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    NSEventType::LeftMouseDragged,
                    position,
                    NSEventModifierFlags::empty(),
                    timestamp,
                    ns_window.windowNumber(),
                    None,
                    0,
                    1,
                    1.0,
                ) else {
                    return failed("could not build the drag event".into());
                };

                let source = PromiseDragSource::new(on_event, mtm);
                LIVE.with(|live| {
                    let mut live = live.borrow_mut();
                    while live.len() >= LIVE_DELEGATES {
                        live.remove(0);
                    }
                    live.push(delegate);
                });
                // The content view owns the returned session; the delegate
                // callbacks report how it goes.
                drop(content_view.beginDraggingSessionWithItems_event_source(
                    &NSArray::from_retained_slice(&[item]),
                    &event,
                    ProtocolObject::from_ref(&*source),
                ));
            })
            .map_err(err)
    }

    /// Settles `token` on the calling thread, which must be the one the
    /// write was requested on (the main thread; see `PENDING`).
    fn settle(token: &str, error: Option<String>) {
        let Some((_, completion)) = PENDING.with(|pending| pending.borrow_mut().remove(token))
        else {
            return;
        };
        match error {
            None => completion.call((std::ptr::null_mut(),)),
            Some(message) => fail(&completion, &message),
        }
    }

    pub fn finish(
        window: &tauri::WebviewWindow,
        token: String,
        error: Option<String>,
    ) -> Result<()> {
        window
            .run_on_main_thread(move || settle_then_sweep(&token, error))
            .map_err(err)
    }

    /// Settles `token`, and only then fails whatever else has waited too long.
    ///
    /// The order is what makes a long download work: `finish` arrives when
    /// the file is already written, and a link slow enough to need an hour is
    /// exactly the case the deadline cannot judge. Sweeping first would fail
    /// a download that has just succeeded — and `finish` would still report
    /// success to the page.
    fn settle_then_sweep(token: &str, error: Option<String>) {
        settle(token, error);
        sweep_expired();
    }

    #[cfg(test)]
    mod tests {
        use std::sync::{Arc, Mutex};

        use tauri::ipc::InvokeResponseBody;

        use super::*;

        /// A channel that keeps what was sent, as the JSON the Filer gets.
        fn recording_channel() -> (Channel<PromisedDragEvent>, Arc<Mutex<Vec<String>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&seen);
            let channel = Channel::new(move |body| {
                if let InvokeResponseBody::Json(json) = body {
                    sink.lock().unwrap().push(json);
                }
                Ok(())
            });
            (channel, seen)
        }

        /// What the receiver's completion block was called with: `None` until
        /// it is called, then the error's description or `None` for success.
        type Outcome = Arc<Mutex<Option<Option<String>>>>;

        fn completion_block() -> (Completion, Outcome) {
            let outcome: Outcome = Arc::new(Mutex::new(None));
            let sink = Arc::clone(&outcome);
            let block = RcBlock::new(move |error: *mut NSError| {
                // SAFETY: AppKit's contract for the block is a live NSError or
                // null, and `fail` keeps its error alive across the call.
                let message = (!error.is_null())
                    .then(|| unsafe { &*error }.localizedDescription().to_string());
                *sink.lock().unwrap() = Some(message);
            });
            (block, outcome)
        }

        /// Asks the delegate the way a receiver does: through the provider's
        /// Objective-C protocol, so the class registration is exercised too.
        /// Tests run off the main thread; nothing here reaches the UI.
        fn request_write(destination: &str) -> (String, Outcome, Arc<Mutex<Vec<String>>>) {
            let mtm = unsafe { MainThreadMarker::new_unchecked() };
            let (channel, seen) = recording_channel();
            let delegate = PromiseDelegate::new("report.txt".into(), channel, mtm);
            let file_type = NSString::from_str("public.data");
            let provider = NSFilePromiseProvider::initWithFileType_delegate(
                NSFilePromiseProvider::alloc(),
                &file_type,
                ProtocolObject::from_ref(&*delegate),
            );
            let receiver_side = provider
                .delegate()
                .expect("the provider keeps its delegate");
            assert_eq!(
                receiver_side
                    .filePromiseProvider_fileNameForType(&provider, &file_type, mtm)
                    .to_string(),
                "report.txt"
            );

            let (block, outcome) = completion_block();
            let url = NSURL::fileURLWithPath(&NSString::from_str(destination));
            receiver_side
                .filePromiseProvider_writePromiseToURL_completionHandler(&provider, &url, &block);

            let events = seen.lock().unwrap().clone();
            assert_eq!(events.len(), 1, "one write request: {events:?}");
            let event: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
            assert_eq!(event["kind"], "write");
            assert_eq!(event["destination"], destination);
            let token = event["token"].as_str().expect("a token").to_string();
            assert!(
                outcome.lock().unwrap().is_none(),
                "the receiver has to wait for the download"
            );
            (token, outcome, seen)
        }

        #[test]
        fn a_finished_download_settles_the_promise_without_an_error() {
            let (token, outcome, _) = request_write("/private/tmp/promised/report.txt");
            settle(&token, None);
            assert_eq!(*outcome.lock().unwrap(), Some(None));
        }

        #[test]
        fn a_failed_download_tells_the_receiver_why() {
            let (token, outcome, _) = request_write("/private/tmp/promised/report.txt");
            settle(&token, Some("report.txt: download cancelled".into()));
            assert_eq!(
                *outcome.lock().unwrap(),
                Some(Some("report.txt: download cancelled".into()))
            );
        }

        #[test]
        fn a_settled_token_is_not_settled_twice() {
            let (token, outcome, _) = request_write("/private/tmp/promised/report.txt");
            settle(&token, None);
            settle(&token, Some("late".into()));
            settle("never-issued", None);
            assert_eq!(*outcome.lock().unwrap(), Some(None));
        }

        /// Pretends the receiver asked for the file `by` earlier than it did.
        fn age(token: &str, by: Duration) {
            PENDING.with(|pending| {
                if let Some((asked, _)) = pending.borrow_mut().get_mut(token) {
                    *asked = Instant::now() - by;
                }
            });
        }

        /// A download that outlived the deadline did produce its file, so
        /// finishing it has to settle the promise rather than fail it: the
        /// receiver is told the file is there, not that it never came.
        #[test]
        fn a_download_that_outlives_the_deadline_still_succeeds() {
            let (token, outcome, _) = request_write("/private/tmp/promised/report.txt");
            age(&token, WRITE_DEADLINE + Duration::from_secs(1));

            settle_then_sweep(&token, None);

            assert_eq!(*outcome.lock().unwrap(), Some(None));
            assert!(
                !PENDING.with(|pending| pending.borrow().contains_key(&token)),
                "the block is released"
            );
        }

        /// A page that never reports the download must not leave the receiver
        /// waiting: the write is failed once the deadline has passed.
        #[test]
        fn a_write_nobody_reports_is_failed_after_the_deadline() {
            let (token, outcome, _) = request_write("/private/tmp/promised/report.txt");
            assert!(
                outcome.lock().unwrap().is_none(),
                "the receiver waits first"
            );

            std::thread::sleep(WRITE_DEADLINE + Duration::from_millis(50));
            sweep_expired();

            let message = outcome.lock().unwrap().clone().expect("settled");
            assert!(
                message.is_some_and(|message| message.contains("did not start")),
                "{message:?}"
            );
            assert!(
                !PENDING.with(|pending| pending.borrow().contains_key(&token)),
                "the block is released"
            );
        }
    }
}
