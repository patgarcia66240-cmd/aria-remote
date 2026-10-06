"""Appairage par code à 6 chiffres (docs/REMOTE_ARCHITECTURE.md, sections 5.1 et 6) : le code est TEMPORAIRE (30 minutes par défaut, réglable), à
usage unique et ne vaut pas identité. Un million de codes seulement : sans limite d'essais on les parcourrait vite, donc des échecs répétés
verrouillent (5 échecs = 1 minute de blocage, et un seul code valide par appareil à la fois)."""
from __future__ import annotations

import secrets
import time
from collections.abc import Callable

from config import settings

from .errors import PairingFailed, PairingLocked

DEFAULT_TTL = 1800.0      # un code vit 30 minutes par défaut ; réglable par REMOTE_PAIRING_CODE_TTL (backend/.env)
MIN_TTL, MAX_TTL = 60.0, 86400.0
MAX_FAILURES = 5          # échecs consécutifs avant verrouillage
LOCK_SECONDS = 60.0


class PairingBook:
    def __init__(self, clock: Callable[[], float] = time.monotonic, *, ttl: float | None = None, max_failures: int = MAX_FAILURES,
                 lock_seconds: float = LOCK_SECONDS) -> None:
        self._clock = clock
        # Durée bornée entre 1 minute et 24 h : un réglage absurde ne rend ni les codes inutilisables ni éternels.
        wanted = float(settings.REMOTE_PAIRING_CODE_TTL) if ttl is None else ttl
        self.ttl = min(MAX_TTL, max(MIN_TTL, wanted))
        self.max_failures, self.lock_seconds = max_failures, lock_seconds
        self._codes: dict[str, tuple[str, float]] = {}      # code -> (device_id, expiration)
        self._failures = 0
        self._locked_until = 0.0

    def issue(self, device_id: str) -> str:
        """Nouveau code pour l'appareil (l'ancien est invalidé)."""
        self._codes = {c: v for c, v in self._codes.items() if v[0] != device_id}
        while True:
            code = f"{secrets.randbelow(1_000_000):06d}"
            if code not in self._codes:
                break
        self._codes[code] = (device_id, self._clock() + self.ttl)
        return code

    def redeem(self, code: str) -> str:
        """Appareil correspondant au code, qui est consommé ; sinon PairingFailed (ou PairingLocked après trop d'échecs)."""
        now = self._clock()
        if now < self._locked_until:
            raise PairingLocked("Trop d'essais : réessaie dans une minute.", headers={"Retry-After": str(int(self._locked_until - now) + 1)})
        self._codes = {c: v for c, v in self._codes.items() if v[1] > now}
        entry = self._codes.pop(code, None)
        if entry is None:
            self._failures += 1
            if self._failures >= self.max_failures:
                self._failures, self._locked_until = 0, now + self.lock_seconds
            raise PairingFailed("Code invalide ou expiré.")
        self._failures = 0
        return entry[0]
