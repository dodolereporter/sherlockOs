//! The few SPA pods a video source gives PipeWire, written by hand: the format it offers and the
//! buffers it wants. The layout is SPA's (`spa/pod/pod.h`): each pod is a size and a type, then
//! its body, padded to 8 bytes; an object is its type and id, then properties (a key, flags, a
//! pod). The numbers are those of `spa/param/*.h` and `spa/utils/type.h`, stable since
//! PipeWire 0.3.

/// Pod types.
const ID: u32 = 3;
const INT: u32 = 4;
const RECTANGLE: u32 = 10;
const FRACTION: u32 = 11;
const OBJECT: u32 = 15;
const CHOICE: u32 = 19;

/// Object types and ids.
const OBJECT_FORMAT: u32 = 0x40003;
const OBJECT_PARAM_BUFFERS: u32 = 0x40004;
/// `SPA_PARAM_EnumFormat`, `SPA_PARAM_Format` and `SPA_PARAM_Buffers`.
pub const PARAM_ENUM_FORMAT: u32 = 3;
pub const PARAM_FORMAT: u32 = 4;
const PARAM_BUFFERS: u32 = 5;

/// Keys of a format.
const FORMAT_MEDIA_TYPE: u32 = 1;
const FORMAT_MEDIA_SUBTYPE: u32 = 2;
const FORMAT_VIDEO_FORMAT: u32 = 0x20001;
const FORMAT_VIDEO_SIZE: u32 = 0x20003;
const FORMAT_VIDEO_FRAMERATE: u32 = 0x20004;
const FORMAT_VIDEO_MAX_FRAMERATE: u32 = 0x20005;
const MEDIA_TYPE_VIDEO: u32 = 2;
const MEDIA_SUBTYPE_RAW: u32 = 1;
/// `SPA_VIDEO_FORMAT_BGRx`: blue first in memory, as `xrgb8888` of Wayland.
const VIDEO_FORMAT_BGRX: u32 = 8;

/// Keys of the buffers.
const BUFFERS_BUFFERS: u32 = 1;
const BUFFERS_BLOCKS: u32 = 2;
const BUFFERS_SIZE: u32 = 3;
const BUFFERS_STRIDE: u32 = 4;
const BUFFERS_DATA_TYPE: u32 = 6;
/// `SPA_DATA_MemPtr` and `SPA_DATA_MemFd`.
const DATA_MEM_PTR: u32 = 1;
const DATA_MEM_FD: u32 = 2;

/// Choice types.
const CHOICE_RANGE: u32 = 1;
const CHOICE_FLAGS: u32 = 4;

/// The highest frame rate offered.
pub const MAX_FRAMERATE: u32 = 60;

/// A pod under construction: bytes in the machine's order, as PipeWire reads them.
#[derive(Default)]
struct Pod(Vec<u8>);

impl Pod {
    fn word(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_ne_bytes());
    }

    fn pad(&mut self) {
        while !self.0.len().is_multiple_of(8) {
            self.0.push(0);
        }
    }

    /// A pod of `kind` whose body is `words`.
    fn value(&mut self, kind: u32, words: &[u32]) {
        self.word(4 * words.len() as u32);
        self.word(kind);
        for w in words {
            self.word(*w);
        }
        self.pad();
    }

    /// A choice of `choice` over values of `kind`, each of `words` words.
    fn choice(&mut self, choice: u32, kind: u32, values: &[&[u32]]) {
        let child = values.first().map_or(0, |v| 4 * v.len() as u32);
        self.word(16 + child * values.len() as u32);
        self.word(CHOICE);
        self.word(choice);
        self.word(0);
        self.word(child);
        self.word(kind);
        for value in values {
            for w in *value {
                self.word(*w);
            }
        }
        self.pad();
    }

    /// The key of a property, without flags; its pod follows.
    fn key(&mut self, key: u32) {
        self.word(key);
        self.word(0);
    }

    /// An object of `kind` and `id` whose properties `body` writes.
    fn object(kind: u32, id: u32, body: impl FnOnce(&mut Pod)) -> Vec<u8> {
        let mut props = Pod::default();
        props.word(kind);
        props.word(id);
        body(&mut props);
        let mut pod = Pod::default();
        pod.word(props.0.len() as u32);
        pod.word(OBJECT);
        pod.0.extend(props.0);
        pod.pad();
        pod.0
    }
}

/// The format a source of `width` × `height` offers, `id` being `PARAM_ENUM_FORMAT` or
/// `PARAM_FORMAT`: raw video, BGRx, that size, a variable rate of up to `MAX_FRAMERATE`.
pub fn video_format(id: u32, width: u32, height: u32) -> Vec<u8> {
    Pod::object(OBJECT_FORMAT, id, |p| {
        p.key(FORMAT_MEDIA_TYPE);
        p.value(ID, &[MEDIA_TYPE_VIDEO]);
        p.key(FORMAT_MEDIA_SUBTYPE);
        p.value(ID, &[MEDIA_SUBTYPE_RAW]);
        p.key(FORMAT_VIDEO_FORMAT);
        p.value(ID, &[VIDEO_FORMAT_BGRX]);
        p.key(FORMAT_VIDEO_SIZE);
        p.value(RECTANGLE, &[width, height]);
        // Variable: a frame comes when the screen changes.
        p.key(FORMAT_VIDEO_FRAMERATE);
        p.value(FRACTION, &[0, 1]);
        p.key(FORMAT_VIDEO_MAX_FRAMERATE);
        let rates: [&[u32]; 3] = [&[MAX_FRAMERATE, 1], &[1, 1], &[MAX_FRAMERATE, 1]];
        p.choice(CHOICE_RANGE, FRACTION, &rates);
    })
}

/// The buffers of frames of `height` rows of `stride` bytes: 2 to 8 of them, 4 by default, in
/// shared memory that PipeWire allocates and maps.
pub fn buffers(stride: u32, height: u32) -> Vec<u8> {
    Pod::object(OBJECT_PARAM_BUFFERS, PARAM_BUFFERS, |p| {
        p.key(BUFFERS_BUFFERS);
        p.choice(CHOICE_RANGE, INT, &[&[4], &[2], &[8]]);
        p.key(BUFFERS_BLOCKS);
        p.value(INT, &[1]);
        p.key(BUFFERS_SIZE);
        p.value(INT, &[stride.saturating_mul(height)]);
        p.key(BUFFERS_STRIDE);
        p.value(INT, &[stride]);
        p.key(BUFFERS_DATA_TYPE);
        let types = (1 << DATA_MEM_FD) | (1 << DATA_MEM_PTR);
        p.choice(CHOICE_FLAGS, INT, &[&[types]]);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(bytes: &[u8]) -> Vec<u32> {
        let (words, rest) = bytes.as_chunks::<4>();
        assert!(rest.is_empty());
        words.iter().map(|w| u32::from_ne_bytes(*w)).collect()
    }

    /// The format, word by word, as `spa_pod_builder` writes it for the same values (the
    /// layout of `spa/pod/builder.h`).
    #[test]
    fn the_format_is_laid_out_as_spa_writes_it() {
        let pod = words(&video_format(PARAM_ENUM_FORMAT, 1920, 1080));
        let expected: Vec<u32> = [
            // The object: its size, its type, then the object type and id.
            &[
                pod.len() as u32 * 4 - 8,
                OBJECT,
                OBJECT_FORMAT,
                PARAM_ENUM_FORMAT,
            ][..],
            // Media type and subtype: an id each, padded to 8 bytes.
            &[1, 0, 4, ID, 2, 0],
            &[2, 0, 4, ID, 1, 0],
            &[0x20001, 0, 4, ID, 8, 0],
            &[0x20003, 0, 8, RECTANGLE, 1920, 1080],
            &[0x20004, 0, 8, FRACTION, 0, 1],
            // A range of fractions: default, minimum, maximum.
            &[0x20005, 0, 40, CHOICE, CHOICE_RANGE, 0, 8, FRACTION],
            &[60, 1, 1, 1, 60, 1],
        ]
        .concat();
        assert_eq!(pod, expected);
    }

    /// Every pod ends on 8 bytes, the choice of 3 ints included (28 bytes, padded).
    #[test]
    fn the_buffers_are_padded_to_eight_bytes() {
        let bytes = buffers(1920 * 4, 1080);
        assert_eq!(bytes.len() % 8, 0);
        let pod = words(&bytes);
        assert_eq!(
            &pod[..4],
            &[pod.len() as u32 * 4 - 8, OBJECT, OBJECT_PARAM_BUFFERS, 5]
        );
        // The count: a range of ints, 28 bytes of body, then 4 bytes of padding.
        assert_eq!(
            &pod[4..14],
            &[1, 0, 28, CHOICE, CHOICE_RANGE, 0, 4, INT, 4, 2]
        );
        assert_eq!(&pod[14..17], &[8, 0, 2]);
        let size = pod.windows(4).any(|w| w == [3, 0, 4, INT]);
        assert!(size);
        assert!(pod.contains(&(1920 * 4 * 1080)));
    }
}
