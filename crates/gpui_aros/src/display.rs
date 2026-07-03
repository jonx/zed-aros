//! The single synthetic AROS display.

use anyhow::Result;
use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, Point, Size, px};
use uuid::Uuid;

use crate::glue;

// Stable identifier for the one display we report.
const AROS_DISPLAY_UUID: Uuid = Uuid::from_u128(0xa2050d15_91a4_0000_0000_000000000001);

#[derive(Debug)]
pub(crate) struct ArosDisplay {
    id: DisplayId,
    bounds: Bounds<Pixels>,
}

impl ArosDisplay {
    pub(crate) fn new() -> Self {
        let (mut w, mut h) = (0i32, 0i32);
        // SAFETY: gpa_screen_size writes both out-params or returns nonzero.
        let ok = unsafe { glue::gpa_screen_size(&mut w, &mut h) } == 0;
        let (width, height) = if ok && w > 0 && h > 0 {
            (w as f32, h as f32)
        } else {
            (1280.0, 800.0)
        };

        Self {
            id: DisplayId::new(1),
            bounds: Bounds {
                origin: Point::default(),
                size: Size {
                    width: px(width),
                    height: px(height),
                },
            },
        }
    }
}

impl PlatformDisplay for ArosDisplay {
    fn id(&self) -> DisplayId {
        self.id
    }

    fn uuid(&self) -> Result<Uuid> {
        Ok(AROS_DISPLAY_UUID)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
}
