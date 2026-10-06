//! Souris : coordonnées normalisées (0..1) du contrôleur -> pixels de l'écran, boutons du navigateur (MouseEvent.button) -> boutons.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

impl Button {
    pub fn from_dom(button: u8) -> Option<Self> {
        match button {
            0 => Some(Self::Left),
            1 => Some(Self::Middle),
            2 => Some(Self::Right),
            3 => Some(Self::Back),
            4 => Some(Self::Forward),
            _ => None,
        }
    }
}

/// Position en pixels, toujours DANS l'écran même si le contrôleur envoie n'importe quoi (NaN, négatif, > 1).
pub fn to_pixels(x: f64, y: f64, width: u32, height: u32) -> (i32, i32) {
    let scale = |value: f64, size: u32| -> i32 {
        let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
        (value * f64::from(size.saturating_sub(1))).round() as i32
    };
    (scale(x, width), scale(y, height))
}

/// Molette du navigateur (pixels, bornés) -> crans : 100 px = 1 cran, même sens (positif = vers le bas).
pub fn wheel_steps(delta: f64) -> i32 {
    if !delta.is_finite() { return 0; }
    (delta.clamp(-400.0, 400.0) / 100.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_stay_inside_the_screen() {
        assert_eq!(to_pixels(0.0, 0.0, 1920, 1080), (0, 0));
        assert_eq!(to_pixels(1.0, 1.0, 1920, 1080), (1919, 1079));
        assert_eq!(to_pixels(-5.0, 9.0, 1920, 1080), (0, 1079));
        assert_eq!(to_pixels(f64::NAN, f64::INFINITY, 1920, 1080), (0, 0));
        assert_eq!(to_pixels(0.5, 0.5, 0, 0), (0, 0));
    }

    #[test]
    fn buttons_follow_the_dom_numbering() {
        assert_eq!(Button::from_dom(0), Some(Button::Left));
        assert_eq!(Button::from_dom(2), Some(Button::Right));
        assert_eq!(Button::from_dom(5), None);
    }

    #[test]
    fn the_wheel_is_converted_to_bounded_steps() {
        assert_eq!(wheel_steps(100.0), 1);
        assert_eq!(wheel_steps(-9999.0), -4);
        assert_eq!(wheel_steps(10.0), 0);
        assert_eq!(wheel_steps(f64::NAN), 0);
    }
}
