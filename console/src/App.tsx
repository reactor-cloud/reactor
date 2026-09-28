import { BrowserRouter, Route, Routes } from "react-router-dom"
import { Data } from "@/data"
import { Logs } from "@/logs"
import { Cluster } from "@/cluster"
import { Operators } from "@/operators"
import { Overview } from "@/overview"
import { Functions } from "@/functions"
import { Email } from "@/email"
import { Home, Keys, Login, Setup, Team, UserDetail, Users } from "@/pages"
import { Settings } from "@/settings"
import { Sites } from "@/sites"
import { Shell } from "@/shell"
import { Storage } from "@/storage"

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
          <Route path="p/:ref/auth" element={<Users />} />
          <Route path="p/:ref/auth/:userId" element={<UserDetail />} />
          <Route path="p/:ref/email" element={<Email />} />
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
