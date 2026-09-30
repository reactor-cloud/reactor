import { BrowserRouter, Navigate, Route, Routes, useParams } from "react-router-dom"
import { Data } from "@/data"
import { Logs } from "@/logs"
import { Cluster } from "@/cluster"
import { Operators } from "@/operators"
import { Overview } from "@/overview"
import { Functions } from "@/functions"
import { Auth, MfaSettings, Providers } from "@/auth"
import { EmailSettings, Templates } from "@/email"
import { Home, Keys, Login, Setup, Team, UserDetail, Users } from "@/pages"
import { Settings } from "@/settings"
import { Sites } from "@/sites"
import { Shell } from "@/shell"
import { Storage } from "@/storage"

function EmailRedirect() {
  const { ref = "" } = useParams()
  return <Navigate to={`/p/${ref}/auth/email`} replace />
}

export default function App() {
  return (
    <BrowserRouter basename="/console">
      <Routes>
        <Route path="/setup" element={<Setup />} />
        <Route path="/login" element={<Login />} />
        <Route element={<Shell />}>
          <Route index element={<Home />} />
          <Route path="cluster" element={<Cluster />} />
          <Route path="users" element={<Operators />} />
          <Route path="p/:ref" element={<Overview />} />
          <Route path="p/:ref/keys" element={<Keys />} />
          <Route path="p/:ref/auth" element={<Auth />}>
            <Route index element={<Users />} />
            <Route path="users/:userId" element={<UserDetail />} />
            <Route path="providers" element={<Providers />} />
            <Route path="mfa" element={<MfaSettings />} />
            <Route path="email" element={<EmailSettings />} />
            <Route path="templates" element={<Templates />} />
          </Route>
          <Route path="p/:ref/email" element={<EmailRedirect />} />
          <Route path="p/:ref/data" element={<Data />} />
          <Route path="p/:ref/data/:table" element={<Data />} />
          <Route path="p/:ref/storage" element={<Storage />} />
          <Route path="p/:ref/functions" element={<Functions />} />
          <Route path="p/:ref/functions/:name" element={<Functions />} />
          <Route path="p/:ref/functions/:name/:section" element={<Functions />} />
          <Route path="p/:ref/sites" element={<Sites />} />
          <Route path="p/:ref/sites/:section" element={<Sites />} />
          <Route path="p/:ref/logs" element={<Logs />} />
          <Route path="p/:ref/team" element={<Team />} />
          <Route path="p/:ref/settings" element={<Settings />} />
        </Route>
      </Routes>
    </BrowserRouter>
  )
}
