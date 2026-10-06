"""Routes /api/remote (docs/REMOTE_ARCHITECTURE.md, section 11). La clé d'API est exigée partout (plugin_router) ; les erreurs du plugin sont des
HTTPException : aucune conversion à écrire ici."""
import asyncio
import json

from fastapi import Query, WebSocket, WebSocketDisconnect
from pydantic import BaseModel, Field

from plugin_sdk.errors import PluginError
from plugin_sdk.router import plugin_router

from . import ice, identity
from .gateway import AgentConnection, hub
from .models import ID_PATTERN, MAX_ICE_SIZE, MAX_SDP
from .service import service

SIGNATURE_PATTERN = r"^[A-Za-z0-9+/]{86}==$"      # Ed25519 : 64 octets en base64
AGENT_FRAME_MAX = MAX_SDP + 4096
AGENT_AUTH_TIMEOUT = 15.0

router = plugin_router()
pairing_router = plugin_router(rate_limit_setting="REMOTE_PAIR_RATE_LIMIT")    # limite d'essais : un code n'a que 6 chiffres


class RegisterBody(BaseModel):
    device_id: str = Field(pattern=ID_PATTERN)
    name: str = Field(min_length=1, max_length=60, pattern=r"^[^|\x00-\x1f]+$")
    public_key: str = Field(min_length=43, max_length=44)       # Ed25519, 32 octets en base64
    platform: str = Field(default="windows", pattern=r"^[a-z0-9_-]{2,20}$")
    timestamp: int = Field(ge=0)                                  # secondes Unix
    nonce: str = Field(pattern=identity.NONCE_PATTERN)
    signature: str = Field(pattern=SIGNATURE_PATTERN)             # signature de identity.register_message


class PairBody(BaseModel):
    code: str = Field(pattern=r"^\d{6}$")


class SessionBody(BaseModel):
    device_id: str = Field(pattern=ID_PATTERN)
    permissions: list[str] | None = Field(default=None, max_length=10)


class SdpBody(BaseModel):
    session_id: str = Field(min_length=6, max_length=64)
    sdp: str = Field(min_length=1, max_length=MAX_SDP)


class AnswerBody(SdpBody):
    signature: str = Field(pattern=SIGNATURE_PATTERN)            # signature de identity.answer_message, faite par l'agent


class IceBody(BaseModel):
    session_id: str = Field(min_length=6, max_length=64)
    candidate: str = Field(min_length=1, max_length=MAX_ICE_SIZE)


@router.get("/status")
async def status() -> dict:
    """Le module est-il opérationnel ? Sans agent branché, aucune session ne peut s'ouvrir."""
    active = service.active_session()
    return {"agent_installed": service.agent_installed(), "agents_online": len(hub.connections), "active_session": active.to_dict() if active else None,
            "turn_configured": any("username" in server for server in ice.ice_servers())}


@router.get("/ice-servers")
async def ice_servers(session_id: str = Query(default="controller", pattern=r"^[A-Za-z0-9_-]{1,64}$")) -> dict:
    """Serveurs STUN/TURN à donner à RTCPeerConnection (vide = réseau local seulement)."""
    return {"ice_servers": ice.ice_servers(session_id)}


@pairing_router.post("/devices/register")
async def register_device(body: RegisterBody) -> dict:
    """Côté agent : annonce l'identité de l'appareil et reçoit le code d'appairage à saisir sur le contrôleur."""
    device, code = service.register_device(body.device_id, body.name, body.public_key, body.platform, body.timestamp, body.nonce, body.signature)
    return {"device": device.to_dict(), "pairing_code": code, "expires_in": int(service.pairing.ttl)}


@pairing_router.post("/pair")
async def pair(body: PairBody) -> dict:
    return {"device": service.pair(body.code).to_dict()}


@router.get("/devices")
async def devices() -> dict:
    return {"devices": await service.list_devices()}


@router.get("/devices/{device_id}")
async def device(device_id: str) -> dict:
    found = service.device(device_id)
    return found.to_dict(online=await service.gateway.is_online(device_id))


@router.post("/sessions")
async def create_session(body: SessionBody) -> dict:
    return (await service.create_session(body.device_id, body.permissions)).to_dict()


@router.get("/sessions/{session_id}")
async def get_session(session_id: str) -> dict:
    await service.sweep()
    return service.session(session_id).to_dict()


@router.delete("/sessions/{session_id}")
async def delete_session(session_id: str) -> dict:
    return (await service.close_session(session_id)).to_dict()


@router.post("/sessions/{session_id}/reconnect")
async def reconnect(session_id: str) -> dict:
    """Le lien est tombé : la session passe en RECONNECTING ; le contrôleur renvoie ensuite une offre (délai de grâce d'une minute)."""
    return (await service.reconnect(session_id)).to_dict()


@router.get("/sessions/{session_id}/signaling")
async def signaling(session_id: str, after: int = Query(default=0, ge=0, le=1000)) -> dict:
    """Côté contrôleur : la réponse de l'agent et ses candidats ICE (à partir de `after`, la valeur `next` du précédent appel)."""
    await service.sweep()
    return service.signaling_view(session_id, after)


@router.post("/signaling/offer")
async def offer(body: SdpBody) -> dict:
    return (await service.submit_offer(body.session_id, body.sdp)).to_dict()


@router.post("/signaling/answer")
async def answer(body: AnswerBody) -> dict:
    return service.submit_answer(body.session_id, body.sdp, body.signature).to_dict()


@router.post("/signaling/ice")
async def ice_candidate(body: IceBody) -> dict:
    return (await service.add_ice(body.session_id, body.candidate)).to_dict()


# -- agent : WebSocket ---------------------------------------------------------------------------------------------------------
@router.websocket("/agent/ws")
async def agent_socket(websocket: WebSocket, device_id: str = Query(pattern=ID_PATTERN)) -> None:
    """Canal de l'agent Remote (clé d'API exigée, comme partout). L'agent prouve ensuite qu'il détient la clé privée de l'appareil en signant un
    défi ; sans cela il n'est jamais déclaré « en ligne ». Messages : voir remote-agent/README.md."""
    device = service.store.get(device_id)
    await websocket.accept()
    if device is None or not device.paired:
        await websocket.close(code=4403, reason="Appareil inconnu ou non appairé.")
        return
    nonce = identity.new_nonce()
    await websocket.send_json({"type": "challenge", "nonce": nonce})
    try:
        auth = json.loads(await asyncio.wait_for(websocket.receive_text(), AGENT_AUTH_TIMEOUT))
        identity.verify(device.public_key, identity.agent_auth_message(device_id, nonce), str(auth.get("signature", "")))
    except (asyncio.TimeoutError, ValueError, WebSocketDisconnect, PluginError, AttributeError):
        service.audit.record("AUTHENTICATION_FAILED", device_id=device_id, result="refused", reason="agent_challenge")
        await websocket.close(code=4401, reason="Authentification de l'agent refusée.")
        return

    async def send(message: dict) -> None:
        await websocket.send_json(message)

    connection = AgentConnection(device_id, send, lambda: websocket.close(code=4000, reason="Remplacé par une nouvelle connexion."))
    await hub.attach(connection)
    await websocket.send_json({"type": "ready", "ice_servers": ice.ice_servers(device_id)})
    try:
        while True:
            text = await websocket.receive_text()
            if len(text) > AGENT_FRAME_MAX:
                await websocket.close(code=1009, reason="Message trop gros.")
                return
            try:
                await _agent_message(connection, json.loads(text))
            except (ValueError, AttributeError, TypeError):
                await send({"type": "error", "detail": "Message illisible."})
            except PluginError as error:
                await send({"type": "error", "detail": error.message})
    except WebSocketDisconnect:
        pass
    finally:
        if hub.detach(connection):
            service.agent_lost(device_id)


async def _agent_message(connection: AgentConnection, message: dict) -> None:
    kind, session_id = message.get("type"), str(message.get("session_id", ""))
    device_id = connection.device_id
    if kind == "session_reply":
        if service.session(session_id).device_id == device_id:
            hub.resolve_reply(session_id, bool(message.get("accepted")), str(message.get("reason", ""))[:200])
    elif kind == "answer":
        service.submit_answer(session_id, str(message.get("sdp", "")), str(message.get("signature", "")), device_id)
    elif kind == "ice":
        service.add_agent_ice(session_id, str(message.get("candidate", "")), device_id)
    elif kind == "grant":
        service.set_granted(device_id, [str(p) for p in message.get("permissions", [])])
    elif kind == "bye":
        service.agent_ended(session_id, device_id)
    elif kind != "ping":
        raise ValueError("type inconnu")


router.include_router(pairing_router)
