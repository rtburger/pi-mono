use crate::{
    Component, ImageDimensions, ImageRenderOptions, get_capabilities, get_image_dimensions,
    image_fallback, render_image, truncate_to_width,
};
use std::cell::{Cell, RefCell};

pub struct ImageTheme {
    fallback_color: Box<ImageFallbackColorFn>,
}

type ImageFallbackColorFn = dyn Fn(&str) -> String + Send + Sync + 'static;

impl ImageTheme {
    pub fn new<F>(fallback_color: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            fallback_color: Box::new(fallback_color),
        }
    }

    pub fn fallback_color(&self, text: &str) -> String {
        (self.fallback_color)(text)
    }
}

impl Default for ImageTheme {
    fn default() -> Self {
        Self::new(str::to_owned)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageOptions {
    pub max_width_cells: Option<usize>,
    pub max_height_cells: Option<usize>,
    pub filename: Option<String>,
    pub image_id: Option<u32>,
}

pub struct Image {
    base64_data: String,
    mime_type: String,
    dimensions: ImageDimensions,
    theme: ImageTheme,
    options: ImageOptions,
    image_id: Cell<Option<u32>>,
    cached_lines: RefCell<Option<Vec<String>>>,
    cached_width: Cell<Option<usize>>,
}

impl Image {
    pub fn new(
        base64_data: impl Into<String>,
        mime_type: impl Into<String>,
        theme: ImageTheme,
        options: ImageOptions,
    ) -> Self {
        let base64_data = base64_data.into();
        let mime_type = mime_type.into();
        let dimensions =
            get_image_dimensions(&base64_data, &mime_type).unwrap_or(ImageDimensions {
                width_px: 800,
                height_px: 600,
            });
        Self::with_dimensions(base64_data, mime_type, theme, options, dimensions)
    }

    pub fn with_dimensions(
        base64_data: impl Into<String>,
        mime_type: impl Into<String>,
        theme: ImageTheme,
        options: ImageOptions,
        dimensions: ImageDimensions,
    ) -> Self {
        let image_id = options.image_id;
        Self {
            base64_data: base64_data.into(),
            mime_type: mime_type.into(),
            dimensions,
            theme,
            options,
            image_id: Cell::new(image_id),
            cached_lines: RefCell::new(None),
            cached_width: Cell::new(None),
        }
    }

    pub fn image_id(&self) -> Option<u32> {
        self.image_id.get()
    }

    pub fn clear_cache(&self) {
        *self.cached_lines.borrow_mut() = None;
        self.cached_width.set(None);
    }
}

impl Component for Image {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        if self.cached_width.get() == Some(width)
            && let Some(cached_lines) = self.cached_lines.borrow().as_ref()
        {
            return cached_lines.clone();
        }

        let max_width = self
            .options
            .max_width_cells
            .unwrap_or(60)
            .min(width.saturating_sub(2).max(1));

        let lines = if get_capabilities().images.is_some() {
            if let Some((sequence, result)) = render_image(
                &self.base64_data,
                self.dimensions,
                ImageRenderOptions {
                    max_width_cells: Some(max_width),
                    max_height_cells: self.options.max_height_cells,
                    preserve_aspect_ratio: true,
                    image_id: self.image_id.get(),
                },
            ) {
                self.image_id.set(result.image_id.or(self.image_id.get()));

                let mut lines = Vec::new();
                for _ in 0..result.rows.saturating_sub(1) {
                    lines.push(String::new());
                }
                let move_up = if result.rows > 1 {
                    format!("\x1b[{}A", result.rows - 1)
                } else {
                    String::new()
                };
                lines.push(format!("{move_up}{sequence}"));
                lines
            } else {
                vec![self.render_fallback(width)]
            }
        } else {
            vec![self.render_fallback(width)]
        };

        self.cached_width.set(Some(width));
        let cached = lines.clone();
        *self.cached_lines.borrow_mut() = Some(cached);
        lines
    }

    fn invalidate(&mut self) {
        self.clear_cache();
    }
}

impl Image {
    fn render_fallback(&self, width: usize) -> String {
        let fallback = image_fallback(
            &self.mime_type,
            Some(self.dimensions),
            self.options.filename.as_deref(),
        );
        let styled = self.theme.fallback_color(&fallback);
        truncate_to_width(&styled, width, "", false)
    }
}
