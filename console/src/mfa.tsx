import { useEffect, useState } from "react"
import { QRCodeSVG } from "qrcode.react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"

type CreationOptions = {
  publicKey: PublicKeyCredentialCreationOptionsJSON
}

type RequestOptions = {
  publicKey: PublicKeyCredentialRequestOptionsJSON
}

type PublicKeyCredentialCreationOptionsJSON = {
  challenge: string
  user: { id: string; name: string; displayName: string }
  excludeCredentials?: Array<{ id: string; type: PublicKeyCredentialType; transports?: AuthenticatorTransport[] }>
  rp: PublicKeyCredentialRpEntity
  pubKeyCredParams: PublicKeyCredentialParameters[]
  timeout?: number
  authenticatorSelection?: AuthenticatorSelectionCriteria
  attestation?: AttestationConveyancePreference
}

type PublicKeyCredentialRequestOptionsJSON = {
  challenge: string
  rpId?: string
  allowCredentials?: Array<{ id: string; type: PublicKeyCredentialType; transports?: AuthenticatorTransport[] }>
  timeout?: number
  userVerification?: UserVerificationRequirement
}

function bytes(value: string) {
  const raw = atob(value.replace(/-/g, "+").replace(/_/g, "/"))
  const out = new Uint8Array(raw.length)
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i)
  return out
}

function text(buffer: ArrayBuffer) {
  const view = new Uint8Array(buffer)
  let raw = ""
  for (const byte of view) raw += String.fromCharCode(byte)
  return btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "")
}

function bearer(token: string) {
  return { headers: { authorization: `Bearer ${token}` } }
}

async function createPasskey(options: CreationOptions) {
  const publicKey: PublicKeyCredentialCreationOptions = {
    ...options.publicKey,
    challenge: bytes(options.publicKey.challenge),
    user: { ...options.publicKey.user, id: bytes(options.publicKey.user.id) },
    excludeCredentials: options.publicKey.excludeCredentials?.map((item) => ({
      ...item,
      id: bytes(item.id),
    })),
  }
  const cred = await navigator.credentials.create({ publicKey })
  if (!(cred instanceof PublicKeyCredential)) throw new Error("passkey was not created")
  const response = cred.response as AuthenticatorAttestationResponse
  return {
    id: cred.id,
    rawId: text(cred.rawId),
    type: cred.type,
    response: {
      attestationObject: text(response.attestationObject),
      clientDataJSON: text(response.clientDataJSON),
    },
    extensions: cred.getClientExtensionResults(),
  }
}

async function getPasskey(options: RequestOptions) {
  const publicKey: PublicKeyCredentialRequestOptions = {
    ...options.publicKey,
    challenge: bytes(options.publicKey.challenge),
    allowCredentials: options.publicKey.allowCredentials?.map((item) => ({
      ...item,
      id: bytes(item.id),
    })),
  }
  const cred = await navigator.credentials.get({ publicKey })
  if (!(cred instanceof PublicKeyCredential)) throw new Error("passkey was not used")
  const response = cred.response as AuthenticatorAssertionResponse
  return {
    id: cred.id,
    rawId: text(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: text(response.authenticatorData),
      clientDataJSON: text(response.clientDataJSON),
      signature: text(response.signature),
      userHandle: response.userHandle ? text(response.userHandle) : null,
    },
    extensions: cred.getClientExtensionResults(),
  }
}

type TotpStart = {
  enrolled?: boolean
  secret?: string
  otpauth?: string
  passkey?: boolean
  can_restart?: boolean
}

export function Enroll({
  token,
  onDone,
  onRestart,
}: {
  token: string
  onDone: (access: string) => void
  onRestart: () => void
}) {
  const [secret, setSecret] = useState("")
  const [otpauth, setOtpauth] = useState("")
  const [enrolled, setEnrolled] = useState(false)
  const [canRestart, setCanRestart] = useState(false)
  const [code, setCode] = useState("")
  const [totp, setTotp] = useState(false)
  const [passkey, setPasskey] = useState(false)
  const [codes, setCodes] = useState<string[] | null>(null)
  const [access, setAccess] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    api<TotpStart>("/console/v1/mfa/totp/start", { method: "POST", ...bearer(token) })
      .then((body) => {
        setEnrolled(body.enrolled === true)
        setCanRestart(body.can_restart === true)
        setPasskey(body.passkey === true)
        setSecret(body.enrolled ? "" : body.secret || "")
        setOtpauth(body.enrolled ? "" : body.otpauth || "")
      })
      .catch((err) => setError(err instanceof Error ? err.message : "could not start enrollment"))
  }, [token])

  async function confirmTotp() {
    setBusy(true)
    setError("")
    try {
      const body = await api<{ totp: boolean; passkey: boolean }>("/console/v1/mfa/totp/confirm", {
        method: "POST",
        body: JSON.stringify({ code }),
        ...bearer(token),
      })
      setTotp(body.totp)
      setPasskey(body.passkey)
    } catch (err) {
      setError(err instanceof Error ? err.message : "code was not accepted")
    } finally {
      setBusy(false)
    }
  }

  async function registerPasskey() {
    setBusy(true)
    setError("")
    try {
      const options = await api<CreationOptions>("/console/v1/mfa/passkey/register/options", {
        method: "POST",
        ...bearer(token),
      })
      const credential = await createPasskey(options)
      const body = await api<{ totp: boolean; passkey: boolean }>("/console/v1/mfa/passkey/register", {
        method: "POST",
        body: JSON.stringify(credential),
        ...bearer(token),
      })
      setTotp(body.totp)
      setPasskey(body.passkey)
    } catch (err) {
      setError(err instanceof Error ? err.message : "passkey was not registered")
    } finally {
      setBusy(false)
    }
  }

  async function resetAuthenticator() {
    setBusy(true)
    setError("")
    try {
      const body = await api<TotpStart>("/console/v1/mfa/totp/reset", { method: "POST", ...bearer(token) })
      setEnrolled(false)
      setTotp(false)
      setPasskey(false)
      setCode("")
      setSecret(body.secret || "")
      setOtpauth(body.otpauth || "")
      setCanRestart(body.can_restart === true)
    } catch (err) {
      setError(err instanceof Error ? err.message : "authenticator was not reset")
    } finally {
      setBusy(false)
    }
  }

  async function restart() {
    setBusy(true)
    setError("")
    try {
      await api("/console/v1/setup/restart", { method: "POST", ...bearer(token) })
      onRestart()
    } catch (err) {
      setError(err instanceof Error ? err.message : "setup was not restarted")
    } finally {
      setBusy(false)
    }
  }

  async function finish() {
    setBusy(true)
    setError("")
    try {
      const body = await api<{ access_token: string; recovery_codes: string[] }>("/console/v1/mfa/finish", {
        method: "POST",
        ...bearer(token),
      })
      setAccess(body.access_token)
      setCodes(body.recovery_codes)
    } catch (err) {
      setError(err instanceof Error ? err.message : "enrollment is not complete")
    } finally {
      setBusy(false)
    }
  }

  if (codes) {
    return (
      <div className="grid gap-4">
        <p className="text-sm text-muted-foreground">
          Save these backup codes. Each one works once, after your password, if the passkey and authenticator are unavailable.
        </p>
        <pre className="overflow-auto rounded-lg bg-muted p-3 font-mono text-sm">{codes.join("\n")}</pre>
        <Button type="button" onClick={() => onDone(access)}>
          Continue
        </Button>
      </div>
    )
  }

  const restartControl = canRestart ? (
    <div className="grid gap-2">
      <Button type="button" variant="outline" disabled={busy} onClick={restart}>
        Restart setup
      </Button>
      <p className="text-sm text-muted-foreground">Deletes this admin account and starts cluster setup over.</p>
    </div>
  ) : null

  if (!totp) {
    return (
      <div className="grid gap-4">
        <p className="text-sm font-medium">Authenticator</p>
        {enrolled ? (
          <p className="text-sm text-muted-foreground">
            This authenticator is already set up. Enter the current 6-digit code from that app.
          </p>
        ) : (
          <>
            {otpauth && (
              <div className="grid justify-items-center gap-2">
                <div className="rounded-lg border bg-white p-3">
                  <QRCodeSVG
                    value={otpauth}
                    size={180}
                    level="M"
                    marginSize={4}
                    bgColor="#ffffff"
                    fgColor="#000000"
                    title="Authenticator QR code"
                  />
                </div>
                <p className="text-sm text-muted-foreground">Scan this with your authenticator app.</p>
              </div>
            )}
            <div className="grid gap-2">
              <Label>Authenticator secret</Label>
              <p className="break-all font-mono text-sm">{secret || "Loading"}</p>
            </div>
          </>
        )}
        <Input value={code} onChange={(event) => setCode(event.target.value)} placeholder="6-digit code" autoComplete="one-time-code" />
        <Button type="button" disabled={busy || code.trim() === ""} onClick={confirmTotp}>
          Confirm authenticator
        </Button>
        {enrolled && !passkey && (
          <div className="grid gap-2">
            <Button type="button" variant="outline" disabled={busy} onClick={resetAuthenticator}>
              Reset authenticator
            </Button>
            <p className="text-sm text-muted-foreground">Removes the saved authenticator and shows a new QR code.</p>
          </div>
        )}
        {restartControl}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </div>
    )
  }

  if (!passkey) {
    return (
      <div className="grid gap-4">
        <p className="text-sm font-medium">Passkey</p>
        <p className="text-sm text-muted-foreground">
          Register a passkey for this account. Open the console on a hostname such as localhost, not an IP address.
        </p>
        <Button type="button" disabled={busy} onClick={registerPasskey}>
          Register passkey
        </Button>
        <div className="grid gap-2">
          <Button type="button" variant="outline" disabled={busy} onClick={resetAuthenticator}>
            Reset authenticator
          </Button>
          <p className="text-sm text-muted-foreground">Removes the saved authenticator and shows a new QR code.</p>
        </div>
        {restartControl}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </div>
    )
  }

  return (
    <div className="grid gap-4">
      <p className="text-sm font-medium">Finish</p>
      <p className="text-sm text-muted-foreground">The authenticator and passkey are saved.</p>
      <Button type="button" disabled={busy} onClick={finish}>
        Finish setup
      </Button>
      {restartControl}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

export function SecondFactor({ token, onDone }: { token: string; onDone: (access: string) => void }) {
  const [code, setCode] = useState("")
  const [recovery, setRecovery] = useState(false)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)

  async function submitCode() {
    setBusy(true)
    setError("")
    try {
      const path = recovery ? "/console/v1/mfa/recovery" : "/console/v1/mfa/totp"
      const body = await api<{ access_token: string }>(path, {
        method: "POST",
        body: JSON.stringify({ code }),
        ...bearer(token),
      })
      onDone(body.access_token)
    } catch (err) {
      setError(err instanceof Error ? err.message : "code was not accepted")
    } finally {
      setBusy(false)
    }
  }

  async function usePasskey() {
    setBusy(true)
    setError("")
    try {
      const options = await api<RequestOptions>("/console/v1/mfa/passkey/options", { method: "POST", ...bearer(token) })
      const credential = await getPasskey(options)
      const body = await api<{ access_token: string }>("/console/v1/mfa/passkey/verify", {
        method: "POST",
        body: JSON.stringify(credential),
        ...bearer(token),
      })
      onDone(body.access_token)
    } catch (err) {
      setError(err instanceof Error ? err.message : "passkey was not accepted")
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-4">
      <p className="text-sm text-muted-foreground">Use the passkey or the authenticator code for this account.</p>
      <Button type="button" disabled={busy} onClick={usePasskey}>
        Use passkey
      </Button>
      <div className="grid gap-2">
        <Label htmlFor="mfa-code">{recovery ? "Backup code" : "Authenticator code"}</Label>
        <Input id="mfa-code" value={code} onChange={(event) => setCode(event.target.value)} autoComplete="one-time-code" />
        <Button type="button" disabled={busy || code.trim() === ""} onClick={submitCode}>
          Continue
        </Button>
        <button type="button" className="text-left text-sm text-muted-foreground underline" onClick={() => setRecovery((value) => !value)}>
          {recovery ? "Use an authenticator code" : "Use a backup code"}
        </button>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}
