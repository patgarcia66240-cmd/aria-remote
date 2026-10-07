"""Registre de PORTS : un plugin dépend d'une interface nommée (« remote.agent_gateway »), jamais d'une implémentation précise.

Le plugin déclare l'interface et une implémentation par défaut sûre (souvent « rien d'installé » : erreur claire) ; un autre module, un
autre plugin ou un test fournit la vraie implémentation avec `ports.provide(...)`. On change de technologie (agent Rust, fournisseur de
SMS, bridge de messagerie…) sans modifier le plugin qui s'en sert."""
from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any


class PortRegistry:
    def __init__(self) -> None:
        self._impls: dict[str, Any] = {}

    def provide(self, name: str, implementation: Any, *, replace: bool = False) -> None:
        if name in self._impls and not replace:
            raise ValueError(f"Le port « {name} » a déjà une implémentation (replace=True pour la remplacer).")
        self._impls[name] = implementation

    def get(self, name: str, default: Any = None) -> Any:
        return self._impls.get(name, default)

    def require(self, name: str) -> Any:
        if name not in self._impls:
            raise LookupError(f"Aucune implémentation fournie pour le port « {name} ».")
        return self._impls[name]

    def names(self) -> list[str]:
        return sorted(self._impls)

    @contextmanager
    def override(self, name: str, implementation: Any) -> Iterator[None]:
        """Remplace temporairement une implémentation (tests) puis la restaure."""
        missing = object()
        before = self._impls.get(name, missing)
        self._impls[name] = implementation
        try:
            yield
        finally:
            if before is missing:
                self._impls.pop(name, None)
            else:
                self._impls[name] = before


ports = PortRegistry()
