import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, test } from 'node:test'
import { build, createServer } from 'vite'
import hotFile from './index.ts'

const temporaryRoots = new Set<string>()

/**
 * Creates an isolated Yarn-style workspace and a nested Vite app root.
 */
async function createWorkspace() {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'sandpost-vite-plugin-hot-file-'))
  temporaryRoots.add(directory)
  await writeFile(path.join(directory, 'package.json'), JSON.stringify({ private: true, workspaces: ['apps/*'] }))
  const appRoot = path.join(directory, 'apps', 'frontend')
  await mkdir(appRoot, { recursive: true })
  await writeFile(path.join(appRoot, 'index.html'), '<main>fixture</main>')
  return { directory, appRoot, publicationPath: path.join(directory, '.vite-hot-file') }
}

/**
 * Starts a real Vite server on an operating-system-assigned loopback port.
 */
async function startServer(root: string) {
  const server = await createServer({
    configFile: false,
    root,
    plugins: [hotFile()],
    logLevel: 'silent',
    server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  return server
}

/**
 * Removes all isolated workspace fixtures after each integration test.
 */
afterEach(async () => {
  await Promise.all([...temporaryRoots].map((directory) => rm(directory, { recursive: true, force: true })))
  temporaryRoots.clear()
})

/**
 * Publishes the resolved port at the discovered workspace root and removes it on close.
 */
test('publishes the actual origin from a nested app root and cleans it up', async () => {
  const workspace = await createWorkspace()
  const server = await startServer(workspace.appRoot)

  try {
    const published = await readFile(workspace.publicationPath, 'utf8')
    const localUrl = server.resolvedUrls?.local?.[0]
    assert.ok(localUrl)
    assert.equal(published, `${new URL(localUrl).origin}\n`)
    assert.match(published, /^http:\/\/127\.0\.0\.1:\d+\n$/)
  } finally {
    await server.close()
  }

  await assert.rejects(readFile(workspace.publicationPath), { code: 'ENOENT' })
})

/**
 * Keeps the newest publication when an older concurrent server closes.
 */
test('an older server closing preserves the newer server publication', async () => {
  const workspace = await createWorkspace()
  const olderServer = await startServer(workspace.appRoot)
  const newerServer = await startServer(workspace.appRoot)

  try {
    const localUrl = newerServer.resolvedUrls?.local?.[0]
    assert.ok(localUrl)
    const newerOrigin = `${new URL(localUrl).origin}\n`
    await olderServer.close()
    assert.equal(await readFile(workspace.publicationPath, 'utf8'), newerOrigin)
    await newerServer.close()
    await assert.rejects(readFile(workspace.publicationPath), { code: 'ENOENT' })
  } finally {
    await Promise.all([olderServer.close(), newerServer.close()])
  }
})

/**
 * Leaves no publication behind when Vite runs its production build hook.
 */
test('does not publish during a production build', async () => {
  const workspace = await createWorkspace()
  await build({
    configFile: false,
    root: workspace.appRoot,
    plugins: [hotFile()],
    logLevel: 'silent',
    build: { write: false },
  })

  await assert.rejects(readFile(workspace.publicationPath), { code: 'ENOENT' })
})
