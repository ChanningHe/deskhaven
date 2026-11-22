import { createContext } from 'react'

import { browserDeps, type ConnectDeps } from '../connect/controller'

/** How the remote page reaches the server and the device; tests and screenshots swap it. */
export const RemoteDepsContext = createContext<ConnectDeps>(browserDeps)
