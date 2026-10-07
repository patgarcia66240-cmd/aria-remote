"""Gestionnaire de chat du plugin Remote : AUCUNE phrase à reconnaître ici, elles vivent dans intents.json (kit commun : plugin_sdk.chat).

Une demande de chat ne contourne rien : elle passe par le même service que l'API (appareil appairé, permissions, agent en ligne, journal)."""
import json

import httpx

from plugin_sdk.chat import load_intents
from plugin_sdk.errors import PluginError

from . import proxy
from .service import service

INTENTS = load_intents(__file__, "remote")


def matches(message: str) -> bool:
    return INTENTS.match(message) is not None


def _reply(text: str) -> dict:
    return {"response": text, "source": "local", "source_type": "remote"}


def _devices_text(devices: list[dict]) -> str:
    return "Appareils : " + ", ".join(f"{d['name']} ({'en ligne' if d['online'] else 'hors ligne'})" for d in devices) + "."


async def _via_rendezvous(found) -> dict:
    """Même demande, mais les appareils et les sessions sont ceux du serveur de rendez-vous (accès par Internet)."""
    def detail(response: httpx.Response) -> str:
        try:
            return str(response.json().get("detail", "")) or f"erreur {response.status_code}"
        except ValueError:
            return f"erreur {response.status_code}"

    try:
        if found.intent == "remote_disconnect":
            status = await proxy.call("GET", "status")
            active = status.json().get("active_session") if status.status_code == 200 else None
            if not active:
                return _reply("Aucune session de contrôle à distance n'est ouverte.")
            await proxy.call("DELETE", f"sessions/{active['session_id']}")
            return _reply("Session coupée.")
        listing = await proxy.call("GET", "devices")
        if listing.status_code != 200:
            return _reply(f"Serveur de rendez-vous : {detail(listing)}")
        devices = listing.json()["devices"]
        if found.intent == "remote_status":
            return _reply(_devices_text(devices) if devices else "Aucun appareil appairé.")
        name = found.slots.get("name", "").strip(" .!?")
        device = next((d for d in devices if d["name"].lower() == name.lower()), None)
        if device is None:
            return _reply(f"Je ne connais pas d'appareil nommé « {name} ». Il doit d'abord être appairé avec son code à 6 chiffres.")
        created = await proxy.call("POST", "sessions", content=json.dumps({"device_id": device["device_id"]}).encode())
        if created.status_code != 200:
            return _reply(detail(created))
        return _reply(f"Connexion à {device['name']} en cours (session {created.json()['session_id']}).")
    except httpx.HTTPError:
        return _reply("Le serveur de rendez-vous est injoignable.")


async def handle(message: str, context: dict) -> dict:
    found = INTENTS.match(message)
    if found is None:
        return _reply("Je n'ai pas compris cette demande de contrôle à distance.")
    if proxy.configured():
        return await _via_rendezvous(found)
    try:
        if found.intent == "remote_status":
            devices = await service.list_devices()
            if not devices:
                return _reply("Aucun appareil appairé. " + ("" if service.agent_installed() else "L'agent Remote n'est pas encore installé."))
            return _reply(_devices_text(devices))
        if found.intent == "remote_disconnect":
            active = service.active_session()
            if active is None:
                return _reply("Aucune session de contrôle à distance n'est ouverte.")
            await service.close_session(active.session_id)
            return _reply("Session coupée.")
        name = found.slots.get("name", "").strip(" .!?")
        device = service.find_device(name)
        if device is None:
            return _reply(f"Je ne connais pas d'appareil nommé « {name} ». Il doit d'abord être appairé avec son code à 6 chiffres.")
        session = await service.create_session(device.device_id)
        return _reply(f"Connexion à {device.name} en cours (session {session.session_id}).")
    except PluginError as error:
        return _reply(error.message)
