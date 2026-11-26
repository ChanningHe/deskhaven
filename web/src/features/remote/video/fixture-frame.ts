import { fromBinary } from '@bufbuild/protobuf'
import nacl from 'tweetnacl'

import { fixtureBytes, fixtureSession, type FixtureVersion } from '../__fixtures__/fixtures'
import { frameNonce } from '../crypto/frame-cipher'
import { MessageSchema, type VideoFrame as VideoFrameMessage } from '../proto/gen/message_pb.js'

const VIDEO_FRAME_SEQ = 6n

export function fixtureKeyFrame(version: FixtureVersion = '1.5.0'): VideoFrameMessage {
  const sealed = fixtureBytes(version, 'hbbr-enc-06-VideoFrame')
  const key = fixtureSession(version).symmetricKey
  const plain = nacl.secretbox.open(sealed, frameNonce(VIDEO_FRAME_SEQ), key)
  if (!plain) throw new Error('fixture VideoFrame does not open')
  const message = fromBinary(MessageSchema, plain)
  if (message.union.case !== 'videoFrame') throw new Error('fixture frame 6 is not a VideoFrame')
  return message.union.value
}
