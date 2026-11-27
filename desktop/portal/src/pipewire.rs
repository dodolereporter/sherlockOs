//! A video source in PipeWire, through `libpipewire-0.3.so.0` loaded at run time (ADR 0028):
//! nothing to build against, no headers nor `bindgen`, and the portal still starts (without
//! screen sharing) on a machine without PipeWire. Only the calls a source needs are declared,
//! with the layouts of PipeWire 0.3 and 1.x (`pipewire/stream.h`, `spa/buffer/buffer.h`).
//!
//! Each stream has a thread loop of its own: PipeWire calls back on that thread, and `push`
//! takes the loop's lock to hand a frame over.

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use libloading::Library;

use crate::pod;

/// `SPA_DIRECTION_OUTPUT`.
const DIRECTION_OUTPUT: u32 = 1;
/// `PW_ID_ANY`.
const ID_ANY: u32 = u32::MAX;
/// `PW_STREAM_FLAG_MAP_BUFFERS | PW_STREAM_FLAG_DRIVER`: PipeWire maps the buffers it
/// allocates, and the source drives the graph, a frame when the screen shows one.
const STREAM_FLAGS: u32 = (1 << 2) | (1 << 3);
/// States of a stream (`enum pw_stream_state`).
const STATE_ERROR: c_int = -1;
const STATE_PAUSED: c_int = 2;
const STATE_STREAMING: c_int = 3;
/// How long PipeWire has to give the stream its node.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("PipeWire is not installed: {0}")]
    Library(#[from] libloading::Error),
    #[error("PipeWire: {0}")]
    Call(&'static str),
    #[error("PipeWire stream: {0}")]
    Stream(String),
}

#[repr(C)]
struct SpaList {
    next: *mut SpaList,
    prev: *mut SpaList,
}

/// `struct spa_hook`, zeroed until PipeWire fills it.
#[repr(C)]
struct SpaHook {
    link: SpaList,
    funcs: *const c_void,
    data: *mut c_void,
    removed: *const c_void,
    private: *mut c_void,
}

type Data = *mut c_void;

/// `struct pw_stream_events`, version 0: the later fields are never read.
#[repr(C)]
struct StreamEvents {
    version: u32,
    destroy: Option<unsafe extern "C" fn(Data)>,
    state_changed: Option<unsafe extern "C" fn(Data, c_int, c_int, *const c_char)>,
    control_info: Option<unsafe extern "C" fn(Data, u32, *const c_void)>,
    io_changed: Option<unsafe extern "C" fn(Data, u32, *mut c_void, u32)>,
    param_changed: Option<unsafe extern "C" fn(Data, u32, *const c_void)>,
    add_buffer: Option<unsafe extern "C" fn(Data, *mut c_void)>,
    remove_buffer: Option<unsafe extern "C" fn(Data, *mut c_void)>,
    process: Option<unsafe extern "C" fn(Data)>,
    drained: Option<unsafe extern "C" fn(Data)>,
}

/// The start of `struct pw_buffer`.
#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut c_void,
    datas: *mut SpaData,
}

#[repr(C)]
struct SpaData {
    kind: u32,
    flags: u32,
    fd: i64,
    mapoffset: u32,
    maxsize: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

/// The calls of `libpipewire-0.3.so.0` in use.
struct Calls {
    _library: Library,
    thread_loop_new: unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void,
    thread_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    thread_loop_start: unsafe extern "C" fn(*mut c_void) -> c_int,
    thread_loop_stop: unsafe extern "C" fn(*mut c_void),
    thread_loop_lock: unsafe extern "C" fn(*mut c_void),
    thread_loop_unlock: unsafe extern "C" fn(*mut c_void),
    thread_loop_destroy: unsafe extern "C" fn(*mut c_void),
    context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    context_connect: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    context_destroy: unsafe extern "C" fn(*mut c_void),
    properties_new: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void,
    stream_new: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void,
    stream_add_listener: unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const StreamEvents, Data),
    stream_connect:
        unsafe extern "C" fn(*mut c_void, u32, u32, u32, *mut *const c_void, u32) -> c_int,
    stream_update_params: unsafe extern "C" fn(*mut c_void, *mut *const c_void, u32) -> c_int,
    stream_get_node_id: unsafe extern "C" fn(*mut c_void) -> u32,
    stream_dequeue_buffer: unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer,
    stream_queue_buffer: unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> c_int,
    stream_destroy: unsafe extern "C" fn(*mut c_void),
    /// Since PipeWire 0.3.34: runs a cycle of the graph that the stream drives.
    stream_trigger_process: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
}

/// Copies the symbol `name` out of `library`.
///
/// # Safety
///
/// `T` must be the type of the C function `name`.
unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, libloading::Error> {
    // SAFETY: the caller gives the function's type; the copy outlives the symbol, not the
    // library, which `Calls` keeps.
    unsafe { library.get::<T>(name).map(|s| *s) }
}

impl Calls {
    fn load() -> Result<Self, Error> {
        // SAFETY: loading PipeWire runs its constructors, which only set up its own state.
        let library = unsafe { Library::new("libpipewire-0.3.so.0") }?;
        // SAFETY: each type below is the one `pipewire/*.h` declares for that name.
        unsafe {
            let init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) =
                symbol(&library, b"pw_init\0")?;
            init(ptr::null_mut(), ptr::null_mut());
            Ok(Self {
                thread_loop_new: symbol(&library, b"pw_thread_loop_new\0")?,
                thread_loop_get_loop: symbol(&library, b"pw_thread_loop_get_loop\0")?,
                thread_loop_start: symbol(&library, b"pw_thread_loop_start\0")?,
                thread_loop_stop: symbol(&library, b"pw_thread_loop_stop\0")?,
                thread_loop_lock: symbol(&library, b"pw_thread_loop_lock\0")?,
                thread_loop_unlock: symbol(&library, b"pw_thread_loop_unlock\0")?,
                thread_loop_destroy: symbol(&library, b"pw_thread_loop_destroy\0")?,
                context_new: symbol(&library, b"pw_context_new\0")?,
                context_connect: symbol(&library, b"pw_context_connect\0")?,
                context_destroy: symbol(&library, b"pw_context_destroy\0")?,
                properties_new: symbol(&library, b"pw_properties_new\0")?,
                stream_new: symbol(&library, b"pw_stream_new\0")?,
                stream_add_listener: symbol(&library, b"pw_stream_add_listener\0")?,
                stream_connect: symbol(&library, b"pw_stream_connect\0")?,
                stream_update_params: symbol(&library, b"pw_stream_update_params\0")?,
                stream_get_node_id: symbol(&library, b"pw_stream_get_node_id\0")?,
                stream_dequeue_buffer: symbol(&library, b"pw_stream_dequeue_buffer\0")?,
                stream_queue_buffer: symbol(&library, b"pw_stream_queue_buffer\0")?,
                stream_destroy: symbol(&library, b"pw_stream_destroy\0")?,
                stream_trigger_process: symbol(&library, b"pw_stream_trigger_process\0").ok(),
                _library: library,
            })
        }
    }
}

/// PipeWire, loaded once for the life of the portal.
fn calls() -> Result<&'static Calls, Error> {
    static CALLS: OnceLock<Result<Calls, String>> = OnceLock::new();
    match CALLS.get_or_init(|| Calls::load().map_err(|e| e.to_string())) {
        Ok(calls) => Ok(calls),
        Err(err) => Err(Error::Stream(err.clone())),
    }
}

/// What the callbacks share with the stream.
struct Shared {
    calls: &'static Calls,
    stream: Mutex<*mut c_void>,
    /// The frame size: the buffers asked for once a format is chosen.
    size: Mutex<(u32, u32)>,
    states: Mutex<Sender<Result<c_int, String>>>,
}

// SAFETY: the stream pointer is only used under the thread loop's lock, or before the loop
// starts and after it stops.
unsafe impl Send for Shared {}
// SAFETY: as above; the rest is behind mutexes.
unsafe impl Sync for Shared {}

static EVENTS: StreamEvents = StreamEvents {
    version: 0,
    destroy: None,
    state_changed: Some(on_state_changed),
    control_info: None,
    io_changed: None,
    param_changed: Some(on_param_changed),
    add_buffer: None,
    remove_buffer: None,
    process: None,
    drained: None,
};

/// # Safety
///
/// `data` is the `Shared` the listener was added with; `error` is null or a C string.
unsafe extern "C" fn on_state_changed(data: Data, _old: c_int, state: c_int, error: *const c_char) {
    // SAFETY: the stream outlives its listener, and `Shared` outlives the stream.
    let shared = unsafe { &*(data as *const Shared) };
    let message = match error.is_null() {
        true => String::from("error"),
        // SAFETY: PipeWire gives a C string that lives for the call.
        false => unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned(),
    };
    let event = match state {
        STATE_ERROR => Err(message),
        state => Ok(state),
    };
    if let Ok(states) = shared.states.lock() {
        let _ = states.send(event);
    }
}

/// # Safety
///
/// `data` is the `Shared` the listener was added with; PipeWire calls it on the loop's thread.
unsafe extern "C" fn on_param_changed(data: Data, id: u32, param: *const c_void) {
    // SAFETY: as in `on_state_changed`.
    let shared = unsafe { &*(data as *const Shared) };
    if id != pod::PARAM_FORMAT || param.is_null() {
        return;
    }
    let (Ok(stream), Ok(size)) = (shared.stream.lock(), shared.size.lock()) else {
        return;
    };
    let (width, height) = *size;
    let buffers = pod::buffers(width * 4, height);
    let mut params = [buffers.as_ptr().cast::<c_void>()];
    // SAFETY: called on the loop's thread, the stream alive; the pod lives for the call.
    unsafe { (shared.calls.stream_update_params)(*stream, params.as_mut_ptr(), 1) };
}

/// A video source of PipeWire: a node that the application which asked for the screen reads.
pub struct Stream {
    calls: &'static Calls,
    thread_loop: NonNull<c_void>,
    context: *mut c_void,
    shared: Arc<Shared>,
    _hook: Box<SpaHook>,
    states: Receiver<Result<c_int, String>>,
    node: u32,
    streaming: bool,
}

// SAFETY: every use of the PipeWire objects takes the thread loop's lock.
unsafe impl Send for Stream {}

/// Takes the thread loop's lock for its life.
struct Locked {
    calls: &'static Calls,
    thread_loop: NonNull<c_void>,
}

impl Locked {
    fn new(stream: &Stream) -> Self {
        let (calls, thread_loop) = (stream.calls, stream.thread_loop);
        // SAFETY: the loop lives as long as the stream, which outlives this guard's use.
        unsafe { (calls.thread_loop_lock)(thread_loop.as_ptr()) };
        Self { calls, thread_loop }
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        // SAFETY: taken in `new`.
        unsafe { (self.calls.thread_loop_unlock)(self.thread_loop.as_ptr()) };
    }
}

impl Stream {
    /// A source named `name` of frames of `width` × `height`, connected; its node id is known.
    pub fn new(name: &str, (width, height): (u32, u32)) -> Result<Self, Error> {
        let calls = calls()?;
        let name = CString::new(name).map_err(|_| Error::Call("name"))?;
        // SAFETY: null properties are allowed; the name is copied.
        let thread_loop =
            unsafe { (calls.thread_loop_new)(c"sherlock-portal".as_ptr(), ptr::null()) };
        let thread_loop = NonNull::new(thread_loop).ok_or(Error::Call("pw_thread_loop_new"))?;
        let (sender, states) = mpsc::channel();
        let shared = Arc::new(Shared {
            calls,
            stream: Mutex::new(ptr::null_mut()),
            size: Mutex::new((width, height)),
            states: Mutex::new(sender),
        });
        let mut stream = Self {
            calls,
            thread_loop,
            context: ptr::null_mut(),
            shared,
            // SAFETY: an all-zero hook is the state PipeWire expects before adding it.
            _hook: Box::new(unsafe { std::mem::zeroed() }),
            states,
            node: ID_ANY,
            streaming: false,
        };
        stream.connect(&name, (width, height))?;
        Ok(stream)
    }

    fn connect(&mut self, name: &CStr, (width, height): (u32, u32)) -> Result<(), Error> {
        let calls = self.calls;
        let thread_loop = self.thread_loop.as_ptr();
        {
            let _locked = Locked::new(self);
            // SAFETY: the loop is alive; the context and the core belong to it.
            unsafe {
                let pw_loop = (calls.thread_loop_get_loop)(thread_loop);
                self.context = (calls.context_new)(pw_loop, ptr::null_mut(), 0);
                if self.context.is_null() {
                    return Err(Error::Call("pw_context_new"));
                }
                let core = (calls.context_connect)(self.context, ptr::null_mut(), 0);
                if core.is_null() {
                    return Err(Error::Call("pw_context_connect: is PipeWire running?"));
                }
                let props = (calls.properties_new)(
                    c"media.class".as_ptr(),
                    c"Video/Source".as_ptr(),
                    c"node.description".as_ptr(),
                    c"Partage d'écran".as_ptr(),
                    ptr::null::<c_char>(),
                );
                let stream = (calls.stream_new)(core, name.as_ptr(), props);
                if stream.is_null() {
                    return Err(Error::Call("pw_stream_new"));
                }
                if let Ok(mut s) = self.shared.stream.lock() {
                    *s = stream;
                }
                let data = Arc::as_ptr(&self.shared) as Data;
                let hook: *mut SpaHook = &mut *self._hook;
                (calls.stream_add_listener)(stream, hook, &EVENTS, data);
                let format = pod::video_format(pod::PARAM_ENUM_FORMAT, width, height);
                let mut params = [format.as_ptr().cast::<c_void>()];
                let connected = (calls.stream_connect)(
                    stream,
                    DIRECTION_OUTPUT,
                    ID_ANY,
                    STREAM_FLAGS,
                    params.as_mut_ptr(),
                    1,
                );
                if connected < 0 {
                    return Err(Error::Call("pw_stream_connect"));
                }
            }
            // SAFETY: the loop is alive.
            if unsafe { (calls.thread_loop_start)(thread_loop) } < 0 {
                return Err(Error::Call("pw_thread_loop_start"));
            }
        }
        // The node exists once the stream is paused.
        loop {
            match self.states.recv_timeout(CONNECT_TIMEOUT) {
                Ok(Ok(STATE_PAUSED | STATE_STREAMING)) => break,
                Ok(Ok(_)) => {}
                Ok(Err(err)) => return Err(Error::Stream(err)),
                Err(_) => return Err(Error::Call("no node")),
            }
        }
        let _locked = Locked::new(self);
        let stream = self.stream_ptr();
        // SAFETY: under the lock, the stream alive.
        self.node = unsafe { (calls.stream_get_node_id)(stream) };
        Ok(())
    }

    fn stream_ptr(&self) -> *mut c_void {
        self.shared.stream.lock().map_or(ptr::null_mut(), |s| *s)
    }

    /// The node the application reads, for the portal's answer.
    pub fn node(&self) -> u32 {
        self.node
    }

    /// Whether the stream failed or ended; reads the states PipeWire gave since the last call.
    pub fn failed(&mut self) -> bool {
        let mut failed = false;
        while let Ok(state) = self.states.try_recv() {
            match state {
                Ok(state) => self.streaming = state == STATE_STREAMING,
                Err(err) => {
                    tracing::warn!("PipeWire stream: {err}");
                    failed = true;
                }
            }
        }
        failed
    }

    /// The frames are now `width` × `height`: the format is offered again.
    pub fn resize(&mut self, (width, height): (u32, u32)) {
        let _locked = Locked::new(self);
        if let Ok(mut size) = self.shared.size.lock() {
            *size = (width, height);
        }
        let format = pod::video_format(pod::PARAM_ENUM_FORMAT, width, height);
        let mut params = [format.as_ptr().cast::<c_void>()];
        // SAFETY: under the lock; the pod lives for the call.
        unsafe { (self.calls.stream_update_params)(self.stream_ptr(), params.as_mut_ptr(), 1) };
    }

    /// Gives a frame: `height` rows of `stride` bytes, BGRx, at the size of the stream. Dropped
    /// when the application reads nothing yet, or holds every buffer.
    pub fn push(&mut self, pixels: &[u8], stride: usize) {
        if self.failed() || !self.streaming {
            return;
        }
        let Ok((width, height)) = self.shared.size.lock().map(|s| *s) else {
            return;
        };
        let row = width as usize * 4;
        let _locked = Locked::new(self);
        let stream = self.stream_ptr();
        // SAFETY: under the lock. The buffer, its data and chunk are PipeWire's until queued;
        // the copy stays within `maxsize`, the mapped size.
        unsafe {
            let buffer = (self.calls.stream_dequeue_buffer)(stream);
            let Some(spa) = buffer.as_ref().and_then(|b| b.buffer.as_ref()) else {
                return;
            };
            if let Some(data) = spa.datas.as_ref().filter(|_| spa.n_datas > 0)
                && !data.data.is_null()
                && let Some(chunk) = data.chunk.as_mut()
            {
                let size = row * height as usize;
                let fits = size <= data.maxsize as usize
                    && pixels.len() >= stride * (height as usize - 1) + row;
                if fits {
                    let target = std::slice::from_raw_parts_mut(data.data.cast::<u8>(), size);
                    for (y, out) in target.chunks_exact_mut(row).enumerate() {
                        out.copy_from_slice(&pixels[y * stride..y * stride + row]);
                    }
                }
                chunk.offset = 0;
                chunk.size = if fits { size as u32 } else { 0 };
                chunk.stride = row as i32;
                chunk.flags = 0;
            }
            (self.calls.stream_queue_buffer)(stream, buffer);
            if let Some(trigger) = self.calls.stream_trigger_process {
                trigger(stream);
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let calls = self.calls;
        let thread_loop = self.thread_loop.as_ptr();
        // SAFETY: the stream is destroyed under the lock, then the loop stopped: no callback
        // runs after. The context, then the loop, go last.
        unsafe {
            {
                let _locked = Locked::new(self);
                let stream = self.stream_ptr();
                if !stream.is_null() {
                    (calls.stream_destroy)(stream);
                }
                if let Ok(mut s) = self.shared.stream.lock() {
                    *s = ptr::null_mut();
                }
            }
            (calls.thread_loop_stop)(thread_loop);
            if !self.context.is_null() {
                (calls.context_destroy)(self.context);
            }
            (calls.thread_loop_destroy)(thread_loop);
        }
    }
}
