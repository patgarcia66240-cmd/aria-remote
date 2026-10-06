"""Routes /api/remote (docs/REMOTE_ARCHITECTURE.md, section 11). La clé d'API est exigée partout (plugin_router) ; les erreurs du plugin sont des
HTTPException : aucune conversion à écrire ici."""
from pydantic import BaseModel, Field

from plugin_sdk.router import plugin_router

from .models import ID_PATTERN, MAX_ICE_SIZE, MAX_SDP
from .service import service

router = plugin_router()
pairing_router = plugin_router(rate_limit_setting="REMOTE_PAIR_RATE_LIMIT")    # limite d'essais : un code n'a que 6 chiffres


class RegisterBody(BaseModel):
    device_id: str = Field(pattern=ID_PATTERN)
    name: str = Field(min_length=1, max_length=60)
    public_key: str = Field(min_length=16, max_length=4096)
    platform: str = Field(default="windows", pattern=r"^[a-z0-9_-]{2,20}$")


class PairBody(BaseModel):
    code: str = Field(pattern=r"^\d{6}$")


class SessionBody(BaseModel):
    device_id: str = Field(pattern=ID_PATTERN)
    permissions: list[str] | None = Field(default=None, max_length=10)


class SdpBody(BaseModel):
    session_id: str = Field(min_length=6, max_length=64)
    sdp: str = Field(min_length=1, max_length=MAX_SDP)


class IceBody(BaseModel):
    session_id: str = Field(min_length=6, max_length=64)
    candidate: str = Field(min_length=1, max_length=MAX_ICE_SIZE)


@router.get("/status")
async def status() -> dict:
    """Le module est-il opérationnel ? Sans agent branché, aucune session ne peut s'ouvrir."""
    active = service.active_session()
    return {"agent_installed": service.agent_installed(), "active_session": active.to_dict() if active else None}


@pairing_router.post("/devices/register")
async def register_device(body: RegisterBody) -> dict:
    """Côté agent : annonce l'identité de l'appareil et reçoit le code d'appairage à saisir sur le contrôleur."""
    device, code = service.register_device(body.device_id, body.name, body.public_key, body.platform)
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
    return service.session(session_id).to_dict()


@router.delete("/sessions/{session_id}")
async def delete_session(session_id: str) -> dict:
    return (await service.close_session(session_id)).to_dict()


@router.post("/signaling/offer")
async def offer(body: SdpBody) -> dict:
    return service.submit_offer(body.session_id, body.sdp).to_dict()


@router.post("/signaling/answer")
async def answer(body: SdpBody) -> dict:
    return service.submit_answer(body.session_id, body.sdp).to_dict()


@router.post("/signaling/ice")
async def ice(body: IceBody) -> dict:
    return service.add_ice(body.session_id, body.candidate).to_dict()


router.include_router(pairing_router)
