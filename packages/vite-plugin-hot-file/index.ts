import { readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { searchForWorkspaceRoot, type Plugin } from 'vite'

/**
 * Creates a Vite dev plugin that publishes and cleans up the local server origin.
 */
export default function hotFile(): Plugin {
  return {
    name: 'vite-plugin-hot-file',
    apply: 'serve',
    /**
     * Publishes the actual local origin after Vite starts listening.
     */
    configureServer(server) {
      const publicationPath = path.join(searchForWorkspaceRoot(server.config.root), '.vite-hot-file')
      const temporaryPath = `${publicationPath}.${process.pid}.tmp`

      server.httpServer?.once('listening', () => {
        const localUrl = server.resolvedUrls?.local[0]
        if (!localUrl) throw new Error('Vite has no local URL for the dev server publication')

        const contents = `${new URL(localUrl).origin}\n`
        writeFileSync(temporaryPath, contents)
        renameSync(temporaryPath, publicationPath)

        /**
         * Removes this server's publication when it still owns the file.
         */
        const removePublication = () => {
          try {
            if (readFileSync(publicationPath, 'utf8') === contents) rmSync(publicationPath)
          } catch (error) {
            if (!(error instanceof Error && 'code' in error && error.code === 'ENOENT')) throw error
          }
        }

        /**
         * Closes Vite before exiting with the conventional interrupt status.
         */
        const interruptServer = () => void server.close().then(() => process.exit(130))

        process.once('exit', removePublication)
        process.once('SIGINT', interruptServer)
        server.httpServer?.once('close', () => {
          process.off('exit', removePublication)
          process.off('SIGINT', interruptServer)
          removePublication()
        })
      })
    },
  }
}
