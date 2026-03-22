use std::path::PathBuf;

static CMD_V_PRESSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
use objc::runtime::Object;
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};

#[cfg(target_os = "macos")]
pub(crate) fn install_paste_monitor() {
    use std::sync::OnceLock;

    extern "C" {
        static _NSConcreteGlobalBlock: std::ffi::c_void;
    }

    #[repr(C)]
    struct BlockDescriptor {
        reserved: u64,
        size: u64,
    }

    #[repr(C)]
    struct GlobalBlock {
        isa: *const std::ffi::c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*const GlobalBlock, *mut Object) -> *mut Object,
        descriptor: *const BlockDescriptor,
    }

    unsafe impl Sync for GlobalBlock {}
    unsafe impl Send for GlobalBlock {}

    unsafe extern "C" fn invoke(_block: *const GlobalBlock, event: *mut Object) -> *mut Object {
        let flags: u64 = msg_send![event, modifierFlags];
        let keycode: u16 = msg_send![event, keyCode];
        if keycode == 9 && (flags & 0x100000) != 0 {
            CMD_V_PRESSED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        event
    }

    static DESCRIPTOR: BlockDescriptor = BlockDescriptor {
        reserved: 0,
        size: std::mem::size_of::<GlobalBlock>() as u64,
    };

    static BLOCK: OnceLock<GlobalBlock> = OnceLock::new();

    let block = BLOCK.get_or_init(|| unsafe {
        GlobalBlock {
            isa: &_NSConcreteGlobalBlock as *const _ as *const std::ffi::c_void,
            flags: 0x10000000i32,
            reserved: 0,
            invoke,
            descriptor: &DESCRIPTOR,
        }
    });

    unsafe {
        let monitor: *mut Object = msg_send![
            class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: (1u64 << 10)
            handler: block as *const GlobalBlock
        ];
        let _ = monitor;
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_paste_monitor() {}

pub(crate) fn take_cmd_v_pressed() -> bool {
    CMD_V_PRESSED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_os = "macos")]
pub(crate) fn read_clipboard() -> Option<String> {
    unsafe {
        let pasteboard: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            return None;
        }
        let ns_string_class = class!(NSString);
        let type_str: *mut Object = msg_send![
            ns_string_class,
            stringWithUTF8String: b"public.utf8-plain-text\0".as_ptr()
        ];
        let content: *mut Object = msg_send![pasteboard, stringForType: type_str];
        if content.is_null() {
            return None;
        }
        let utf8: *const std::os::raw::c_char = msg_send![content, UTF8String];
        if utf8.is_null() {
            return None;
        }
        let cstr = std::ffi::CStr::from_ptr(utf8);
        Some(cstr.to_string_lossy().into_owned())
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn read_clipboard() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
pub(crate) fn save_clipboard_image(log: &mut Vec<String>) -> Option<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let desktop =
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned())).join("Desktop");
    log.push(format!("img_paste: desktop = {}", desktop.display()));

    if let Err(e) = std::fs::create_dir_all(&desktop) {
        log.push(format!("img_paste: create_dir_all failed: {e}"));
    }
    let out_path = desktop.join(format!("pasted-image-{ts}.png"));

    unsafe {
        let pasteboard: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            log.push("img_paste: NSPasteboard is NULL".to_owned());
            return None;
        }
        log.push("img_paste: NSPasteboard OK".to_owned());

        let types: *mut Object = msg_send![pasteboard, types];
        if !types.is_null() {
            let count: usize = msg_send![types, count];
            log.push(format!("img_paste: pasteboard has {count} types"));
            for i in 0..count.min(10) {
                let t: *mut Object = msg_send![types, objectAtIndex: i];
                if !t.is_null() {
                    let utf8: *const std::os::raw::c_char = msg_send![t, UTF8String];
                    if !utf8.is_null() {
                        let s = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
                        log.push(format!("img_paste:   type[{i}] = {s}"));
                    }
                }
            }
        } else {
            log.push("img_paste: pasteboard.types is NULL".to_owned());
        }

        let image_alloc: *mut Object = msg_send![class!(NSImage), alloc];
        log.push(format!("img_paste: NSImage alloc = {:p}", image_alloc));
        let image: *mut Object = msg_send![image_alloc, initWithPasteboard: pasteboard];
        if image.is_null() {
            log.push(
                "img_paste: NSImage initWithPasteboard returned NULL — no image on clipboard"
                    .to_owned(),
            );
            return None;
        }
        log.push(format!("img_paste: NSImage OK ({:p})", image));

        let tiff: *mut Object = msg_send![image, TIFFRepresentation];
        if tiff.is_null() {
            log.push("img_paste: TIFFRepresentation is NULL".to_owned());
            return None;
        }
        let tiff_len: usize = msg_send![tiff, length];
        log.push(format!("img_paste: TIFF data = {tiff_len} bytes"));

        let rep: *mut Object = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        if rep.is_null() {
            log.push("img_paste: NSBitmapImageRep is NULL — saving raw TIFF".to_owned());
            let ptr: *const u8 = msg_send![tiff, bytes];
            let bytes = std::slice::from_raw_parts(ptr, tiff_len);
            let tiff_path = out_path.with_extension("tiff");
            return match std::fs::write(&tiff_path, bytes) {
                Ok(_) => {
                    log.push(format!("img_paste: saved TIFF to {}", tiff_path.display()));
                    Some(tiff_path)
                }
                Err(e) => {
                    log.push(format!("img_paste: fs::write TIFF failed: {e}"));
                    None
                }
            };
        }
        log.push("img_paste: NSBitmapImageRep OK".to_owned());

        let props: *mut Object = msg_send![class!(NSDictionary), dictionary];
        let png_data: *mut Object =
            msg_send![rep, representationUsingType: 4usize properties: props];

        let (data_ptr, data_len, path): (*const u8, usize, PathBuf) = if !png_data.is_null() {
            let len: usize = msg_send![png_data, length];
            log.push(format!("img_paste: PNG data = {len} bytes"));
            let ptr: *const u8 = msg_send![png_data, bytes];
            (ptr, len, out_path)
        } else {
            log.push("img_paste: PNG conversion failed — saving raw TIFF".to_owned());
            let ptr: *const u8 = msg_send![tiff, bytes];
            (ptr, tiff_len, out_path.with_extension("tiff"))
        };

        let bytes = std::slice::from_raw_parts(data_ptr, data_len);
        match std::fs::write(&path, bytes) {
            Ok(_) => {
                log.push(format!("img_paste: saved to {}", path.display()));
                Some(path)
            }
            Err(e) => {
                log.push(format!("img_paste: fs::write failed: {e}"));
                None
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn save_clipboard_image(log: &mut Vec<String>) -> Option<PathBuf> {
    log.push("img_paste: save_clipboard_image — not macOS".to_owned());
    None
}
