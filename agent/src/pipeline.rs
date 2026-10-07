//! Pièces du flux d'images, conçues pour une chose : que l'image affichée chez le contrôleur soit la plus RÉCENTE possible, jamais une vieille image
//! restée dans une file d'attente.
//!
//!  - `Slot` : boîte aux lettres « dernière valeur » entre deux étapes (capture -> encodage -> envoi). Une nouvelle image remplace celle qui n'a pas
//!    encore été prise ; aucune file ne se forme, donc aucun retard accumulé quand une étape est plus lente que la précédente.
//!  - `fingerprint` : empreinte rapide d'une image brute, pour ne pas encoder ni envoyer un écran qui n'a pas changé.
//!  - `Governor` : limite ce qu'on laisse s'accumuler dans le canal d'envoi (quelques images, pas un mégaoctet).
//!  - `Counters` : mesures de la dernière fenêtre (images par seconde, débit, temps de capture et d'encodage, images abandonnées).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use tokio::sync::Notify;

struct SlotState<T> {
    value: Option<T>,
    replaced: u64,
    closed: bool,
}

/// Boîte aux lettres à une seule place. `put` n'attend jamais ; `take_blocking` (fils de travail) et `take_async` (tâche tokio) attendent la valeur.
pub struct Slot<T> {
    state: Mutex<SlotState<T>>,
    ready: Condvar,
    wake: Notify,
}

impl<T> Slot<T> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(SlotState { value: None, replaced: 0, closed: false }), ready: Condvar::new(), wake: Notify::new() })
    }

    /// Dépose une valeur ; une valeur non reprise est écrasée (et comptée).
    pub fn put(&self, value: T) {
        if let Ok(mut state) = self.state.lock() {
            if state.closed {
                return;
            }
            if state.value.replace(value).is_some() {
                state.replaced += 1;
            }
        }
        self.ready.notify_one();
        self.wake.notify_one();
    }

    /// Plus rien ne sera déposé ni repris : réveille tous ceux qui attendent.
    pub fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.ready.notify_all();
        self.wake.notify_waiters();
        self.wake.notify_one();
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().map(|s| s.closed).unwrap_or(true)
    }

    /// Nombre de valeurs écrasées avant d'avoir été prises (étape suivante en retard).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn replaced(&self) -> u64 {
        self.state.lock().map(|s| s.replaced).unwrap_or(0)
    }

    /// Attend la prochaine valeur (fil de travail). None quand la boîte est fermée.
    pub fn take_blocking(&self) -> Option<T> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(value) = state.value.take() {
                return Some(value);
            }
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).ok()?;
        }
    }

    /// Attend la prochaine valeur (tâche asynchrone). None quand la boîte est fermée.
    pub async fn take_async(&self) -> Option<T> {
        loop {
            {
                let mut state = self.state.lock().ok()?;
                if let Some(value) = state.value.take() {
                    return Some(value);
                }
                if state.closed {
                    return None;
                }
            }
            self.wake.notified().await;
        }
    }
}

/// Empreinte 64 bits d'une image brute (quatre voies indépendantes : ~10 Go/s, soit quelques ms pour un écran 1080p). Deux images différentes ont
/// la même empreinte avec une probabilité de l'ordre de 2^-64 : ignorable ; une image identique est reconnue à coup sûr.
pub fn fingerprint(data: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut lanes = [0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4, 0x1234_5678_9abc_def0, 0x0fed_cba9_8765_4321];
    let mut blocks = data.chunks_exact(32);
    for block in &mut blocks {
        for (lane, word) in lanes.iter_mut().zip(block.chunks_exact(8)) {
            let word = u64::from_le_bytes(word.try_into().expect("8 octets"));
            *lane = (*lane ^ word).wrapping_mul(K);
            *lane ^= *lane >> 29;
        }
    }
    let mut hash = lanes.iter().fold(data.len() as u64, |acc, lane| (acc ^ lane).wrapping_mul(K).rotate_left(23));
    for byte in blocks.remainder() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Limite de ce qui peut attendre dans le canal d'envoi. Avant : 1 Mo fixe, soit ~1,6 s de retard sur une liaison montante de 5 Mbit/s avant de
/// commencer à sauter des images. Maintenant : environ deux images et demie (donc ~250 ms de retard au pire à 10 images/s), bornée.
pub struct Governor {
    average: f64,
}

impl Governor {
    pub const MIN: usize = 96 * 1024;
    pub const MAX: usize = 512 * 1024;

    pub fn new() -> Self {
        Self { average: 0.0 }
    }

    /// Taille d'une image réellement envoyée (moyenne glissante).
    pub fn observe(&mut self, bytes: usize) {
        self.average = if self.average == 0.0 { bytes as f64 } else { self.average * 0.8 + bytes as f64 * 0.2 };
    }

    pub fn limit(&self) -> usize {
        ((self.average * 2.5) as usize).clamp(Self::MIN, Self::MAX)
    }
}

/// Mesures de la fenêtre en cours ; `report` les lit et les remet à zéro.
#[derive(Default)]
pub struct Counters {
    pub captured: AtomicU64,
    pub unchanged: AtomicU64,
    pub throttled: AtomicU64,
    pub encoded: AtomicU64,
    pub sent: AtomicU64,
    pub dropped: AtomicU64,
    pub bytes: AtomicU64,
    pub capture_us: AtomicU64,
    pub encode_us: AtomicU64,
}

/// Ce que le contrôleur reçoit toutes les deux secondes (message « stats »).
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub fps: f64,
    pub kbps: u64,
    pub capture_ms: f64,
    pub encode_ms: f64,
    pub dropped: u64,
    pub idle: bool,
}

impl Counters {
    pub fn add(counter: &AtomicU64, value: u64) {
        counter.fetch_add(value, Ordering::Relaxed);
    }

    pub fn report(&self, window_ms: u64) -> Report {
        let take = |c: &AtomicU64| c.swap(0, Ordering::Relaxed);
        let (captured, unchanged, _throttled, encoded, sent, dropped, bytes, capture_us, encode_us) =
            (take(&self.captured), take(&self.unchanged), take(&self.throttled), take(&self.encoded), take(&self.sent), take(&self.dropped), take(&self.bytes), take(&self.capture_us), take(&self.encode_us));
        let seconds = (window_ms.max(1) as f64) / 1000.0;
        let average = |total: u64, count: u64| if count == 0 { 0.0 } else { (total as f64) / (count as f64) / 1000.0 };
        Report {
            fps: (((sent as f64) / seconds) * 10.0).round() / 10.0,
            kbps: ((bytes as f64) * 8.0 / 1000.0 / seconds).round() as u64,
            capture_ms: (average(capture_us, captured) * 10.0).round() / 10.0,
            encode_ms: (average(encode_us, encoded) * 10.0).round() / 10.0,
            dropped,
            idle: sent == 0 && unchanged > 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_new_value_replaces_the_one_nobody_took_and_is_counted() {
        let slot = Slot::new();
        slot.put(1);
        slot.put(2);
        slot.put(3);
        assert_eq!(slot.take_blocking(), Some(3), "seule la plus récente est livrée : aucune file, aucun retard");
        assert_eq!(slot.replaced(), 2);
        slot.put(4);
        assert_eq!(slot.replaced(), 2, "une valeur prise n'est pas « écrasée »");
    }

    #[test]
    fn a_blocked_worker_wakes_up_on_a_value_and_on_close() {
        let slot = Slot::new();
        let reader = { let slot = slot.clone(); std::thread::spawn(move || (slot.take_blocking(), slot.take_blocking())) };
        std::thread::sleep(Duration::from_millis(50));
        slot.put("image");
        std::thread::sleep(Duration::from_millis(50));
        slot.close();
        assert_eq!(reader.join().unwrap(), (Some("image"), None));
        slot.put("après fermeture");
        assert_eq!(slot.take_blocking(), None, "plus rien ne passe une fois fermée");
        assert!(slot.is_closed());
    }

    #[tokio::test]
    async fn the_async_side_gets_values_and_the_close() {
        let slot = Slot::new();
        let waiting = { let slot = slot.clone(); tokio::spawn(async move { let a = slot.take_async().await; let b = slot.take_async().await; (a, b) }) };
        tokio::time::sleep(Duration::from_millis(30)).await;
        slot.put(7);
        tokio::time::sleep(Duration::from_millis(30)).await;
        slot.close();
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap(), (Some(7), None));
    }

    #[tokio::test]
    async fn a_value_put_before_anyone_waits_is_not_lost() {
        let slot = Slot::new();
        slot.put(9);
        assert_eq!(tokio::time::timeout(Duration::from_secs(1), slot.take_async()).await.unwrap(), Some(9));
    }

    #[test]
    fn the_fingerprint_sees_any_change_and_ignores_identical_frames() {
        let mut frame = vec![0u8; 1920 * 1080 * 4];
        let before = fingerprint(&frame);
        assert_eq!(fingerprint(&frame.clone()), before);
        for position in [0usize, 7, 31, 32, 1_000_003, frame.len() - 1] {
            let mut changed = frame.clone();
            changed[position] = 1;
            assert_ne!(fingerprint(&changed), before, "un seul octet modifié à {position}");
        }
        frame[5] = 9;
        assert_ne!(fingerprint(&frame), before);
        assert_ne!(fingerprint(&[1, 2, 3]), fingerprint(&[1, 2, 3, 0]), "la longueur compte");
        assert_ne!(fingerprint(&[]), fingerprint(&[0]));
    }

    #[test]
    fn the_send_queue_limit_follows_the_frame_size_within_bounds() {
        let mut governor = Governor::new();
        assert_eq!(governor.limit(), Governor::MIN, "avant toute mesure : la borne basse");
        for _ in 0..30 { governor.observe(100_000); }
        assert!((240_000..=260_000).contains(&governor.limit()), "{}", governor.limit());
        for _ in 0..60 { governor.observe(1_000_000); }
        assert_eq!(governor.limit(), Governor::MAX, "jamais plus de 512 Ko en attente (l'ancienne limite fixe : 1 Mo)");
        for _ in 0..200 { governor.observe(1_000); }
        assert_eq!(governor.limit(), Governor::MIN);
    }

    #[test]
    fn the_report_summarizes_the_window_then_starts_over() {
        let counters = Counters::default();
        for (field, value) in [(&counters.captured, 20), (&counters.unchanged, 5), (&counters.encoded, 10), (&counters.sent, 8), (&counters.dropped, 2), (&counters.bytes, 500_000), (&counters.capture_us, 60_000), (&counters.encode_us, 150_000)] {
            Counters::add(field, value);
        }
        let report = counters.report(2000);
        assert_eq!(report, Report { fps: 4.0, kbps: 2000, capture_ms: 3.0, encode_ms: 15.0, dropped: 2, idle: false });
        let empty = counters.report(2000);
        assert_eq!((empty.fps, empty.kbps, empty.dropped, empty.idle), (0.0, 0, 0, false));
        Counters::add(&counters.unchanged, 3);
        assert!(counters.report(2000).idle, "rien envoyé parce que l'écran n'a pas changé = écran fixe");
    }
}
