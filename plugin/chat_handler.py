"""Gestionnaire de chat du plugin Remote : AUCUNE phrase à reconnaître ici, elles vivent dans intents.json (kit commun : plugin_sdk.chat).

Une demande de chat ne contourne rien : elle passe par le même service que l'API (appareil appairé, permissions, agent en ligne, journal)."""
from plugin_sdk.chat import load_intents
from plugin_sdk.errors import PluginError

from .service import service

INTENTS = load_intents(__file__, "remote")


def matches(message: str) -> bool:
    return INTENTS.match(message) is not None


def _reply(text: str) -> dict:
    return {"response": text, "source": "local", "source_type": "remote"}


async def handle(message: str, context: dict) -> dict:
    found = INTENTS.match(message)
    if found is None:
        return _reply("Je n'ai pas compris cette demande de contrôle à distance.")
    try:
        if found.intent == "remote_status":
            devices = await service.list_devices()
            if not devices:
                return _reply("Aucun appareil appairé. " + ("" if service.agent_installed() else "L'agent Remote n'est pas encore installé."))
            return _reply("Appareils : " + ", ".join(f"{d['name']} ({'en ligne' if d['online'] else 'hors ligne'})" for d in devices) + ".")
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
