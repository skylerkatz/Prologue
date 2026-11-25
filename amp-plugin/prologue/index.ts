import type {
	PluginAPI,
	PluginThread,
	PluginUI,
	QuickAction,
	ThreadID,
} from '@ampcode/plugin'
import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { appendFile, chmod, lstat, mkdir, readFile, readdir, rename, rm, stat, symlink, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

export const description =
	'Review the orb’s changes in Prologue: starts the Prologue web UI in the Portal pane and turns “Send to Amp” into a thread message for the prologue-review skill.'

const GITHUB_REPO = 'skylerkatz/prologue'
const ASSET = 'prologue-web-x86_64-linux.tar.gz'
const SERVICE = 'prologue'
const START_COMMAND = 'start-review'
/**
 * Bump on every change to this plugin, including its skill: prologue-server compares the
 * running plugin's version with the one its release was built from and offers an update.
 * CI fails a change under amp-plugin/prologue that leaves it unchanged.
 */
const PLUGIN_VERSION = 4
/**
 * Release tags the plugin installs: v* releases, plus the rolling main-<sha> builds of every
 * push to main when PROLOGUE_CHANNEL=main.
 */
const CHANNEL_MAIN = process.env.PROLOGUE_CHANNEL === 'main'
const wantedTag = (tag: string) => tag.startsWith('v') || (CHANNEL_MAIN && tag.startsWith('main-'))
/**
 * Workspace default base ref, such as origin/production: used without asking when a project has
 * no remembered base, and preferred over the auto-detected candidates.
 */
const DEFAULT_BASE = process.env.PROLOGUE_DEFAULT_BASE?.trim() || undefined

interface Bundle {
	server: string
	cli: string
	dist: string
	label: string
}

interface ServiceConfig {
	repo: string
	base: string
	server: string
	webhook: string
	ownerId?: string
	ownerEmail?: string
	pluginVersion?: number
	url?: string
}

interface Settings {
	bases: Record<string, string>
	service?: ServiceConfig
	handledEvents: string[]
}

interface Run {
	code: number
	stdout: string
	stderr: string
}

function run(cmd: string, args: string[], cwd?: string, timeoutMs = 120_000): Promise<Run> {
	return new Promise((resolve) => {
		execFile(cmd, args, { cwd, timeout: timeoutMs, maxBuffer: 32 * 1024 * 1024 }, (error, stdout, stderr) => {
			const code = error ? (typeof error.code === 'number' ? error.code : 1) : 0
			resolve({ code, stdout: String(stdout), stderr: String(stderr || (error && !stderr ? error.message : '')) })
		})
	})
}

const git = (repo: string, ...args: string[]) => run('git', ['-C', repo, ...args])

const shellQuote = (value: string) => `'${value.replaceAll("'", `'\\''`)}'`

/** C631, C632, C633, C634, C640 → "C631–C634, C640". */
function formatCommentIds(ids: number[]): string {
	const sorted = [...new Set(ids)].sort((a, b) => a - b)
	const runs: string[] = []
	for (let i = 0; i < sorted.length; ) {
		let j = i
		while (j + 1 < sorted.length && sorted[j + 1] === sorted[j]! + 1) j++
		runs.push(j > i ? `C${sorted[i]}–C${sorted[j]}` : `C${sorted[i]}`)
		i = j + 1
	}
	return runs.length > 8 ? `${runs.slice(0, 8).join(', ')}, …` : runs.join(', ')
}

/** Mirrors PLUGIN_UPDATE_MESSAGE in prologue-server's amp.rs exactly; keep them in step. */
const PLUGIN_UPDATE_MESSAGE =
	'Update my Prologue Amp plugin to the latest version: clone the plugin repository it is installed in (my personal plugins, unless it is a workspace plugin), replace its `prologue` directory with `amp-plugin/prologue` from the main branch of https://github.com/skylerkatz/prologue, commit, and push. Then reload plugins.'

function reviewMessage(ids: number[], count: number, repo?: string): string {
	const noun = count === 1 ? 'open review comment' : 'open review comments'
	const list = ids.length ? ` (${formatCommentIds(ids)})` : ''
	const where = repo ? ` for ${repo}` : ''
	const each = count === 1 ? 'reply to it' : 'reply to each'
	return `You have ${count} ${noun} in Prologue${list}${where}. Use the prologue-review skill to work through them, ${each} with the prologue CLI, then give me a short summary.`
}

export default async function (amp: PluginAPI) {
	// Prologue runs as an orb service behind a portal; elsewhere this plugin does nothing.
	if (!process.env.AMP_ORB) {
		return
	}

	await amp.registerSkill({ path: 'skills/prologue-review' })

	const cacheDir = amp.system.cacheDirectory
	const settingsPath = join(cacheDir, 'settings.json')
	const workspaceRoot = amp.system.workspaceRoot ? amp.helpers.filePathFromURI(amp.system.workspaceRoot) : process.cwd()

	/** A short activity log in the cache directory, for diagnosing what happened after a click. */
	function log(message: string): void {
		amp.logger.log(message)
		void mkdir(cacheDir, { recursive: true })
			.then(() => appendFile(join(cacheDir, 'prologue.log'), `${new Date().toISOString()} ${message}\n`))
			.catch(() => {})
	}

	async function loadSettings(): Promise<Settings> {
		try {
			const parsed = JSON.parse(await readFile(settingsPath, 'utf8'))
			return { bases: {}, handledEvents: [], ...parsed }
		} catch {
			return { bases: {}, handledEvents: [] }
		}
	}

	let settingsWrite = Promise.resolve()
	function updateSettings(change: (settings: Settings) => void): Promise<void> {
		settingsWrite = settingsWrite.then(async () => {
			const settings = await loadSettings()
			change(settings)
			await mkdir(cacheDir, { recursive: true })
			const tmp = `${settingsPath}.${process.pid}.tmp`
			await writeFile(tmp, JSON.stringify(settings, null, '\t'), { mode: 0o600 })
			await rename(tmp, settingsPath)
		})
		return settingsWrite
	}

	// ── Bundle: the latest GitHub release with the web asset, cached by tag ──

	function bundleIn(dir: string, label: string): Bundle | undefined {
		const server = join(dir, 'prologue-server')
		const cli = join(dir, 'prologue')
		const dist = join(dir, 'dist')
		if (existsSync(server) && existsSync(cli) && existsSync(join(dist, 'index.html'))) {
			return { server, cli, dist, label }
		}
		return undefined
	}

	/** The extracted bundle root: the archive may hold the files directly or in one top-level folder. */
	async function findBundle(dir: string, label: string): Promise<Bundle | undefined> {
		const direct = bundleIn(dir, label)
		if (direct) return direct
		for (const entry of await readdir(dir, { withFileTypes: true }).catch(() => [])) {
			if (entry.isDirectory()) {
				const nested = bundleIn(join(dir, entry.name), label)
				if (nested) return nested
			}
		}
		return undefined
	}

	/** When a cached release was published (from its meta.json), else when it was extracted. */
	async function cachedAt(dir: string): Promise<number> {
		try {
			const meta = JSON.parse(await readFile(join(dir, 'meta.json'), 'utf8'))
			const published = Date.parse(meta.publishedAt ?? '')
			if (!Number.isNaN(published)) return published
		} catch {}
		return (await stat(dir).catch(() => undefined))?.mtimeMs ?? 0
	}

	/** The newest cached bundle by publish date. Never by tag name: main-<sha> tags don't sort. */
	async function newestCachedBundle(): Promise<Bundle | undefined> {
		const root = join(cacheDir, 'releases')
		const dated: { tag: string; at: number }[] = []
		for (const tag of await readdir(root).catch(() => [] as string[])) {
			if (!tag.includes('.download-') && wantedTag(tag)) dated.push({ tag, at: await cachedAt(join(root, tag)) })
		}
		dated.sort((a, b) => b.at - a.at)
		for (const { tag } of dated) {
			const bundle = await findBundle(join(root, tag), tag)
			if (bundle) return bundle
		}
		return undefined
	}

	/** A release with the web asset, and when it was published. */
	interface Release {
		tag: string
		publishedAt: string
		url: string
	}

	const assetURL = (tag: string) => `https://github.com/${GITHUB_REPO}/releases/download/${encodeURIComponent(tag)}/${ASSET}`

	async function githubToken(): Promise<string | undefined> {
		const fromEnv = process.env.GITHUB_TOKEN || process.env.GH_TOKEN
		if (fromEnv) return fromEnv
		const gh = await run('gh', ['auth', 'token'], undefined, 5_000)
		return gh.code === 0 && gh.stdout.trim() ? gh.stdout.trim() : undefined
	}

	/** The REST API: exact assets and dates, but 60 requests/hour per IP without a token. */
	async function newestFromAPI(token?: string): Promise<Release | undefined> {
		const headers: Record<string, string> = { Accept: 'application/vnd.github+json', 'User-Agent': 'amp-prologue-plugin' }
		if (token) headers.Authorization = `Bearer ${token}`
		const response = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases?per_page=100`, {
			headers,
			signal: AbortSignal.timeout(20_000),
		})
		if (!response.ok) {
			const limited = response.headers.get('x-ratelimit-remaining') === '0'
			throw new Error(`GitHub API returned HTTP ${response.status}${limited ? ' (rate limited)' : ''}`)
		}
		const releases = (await response.json()) as {
			tag_name: string
			draft: boolean
			published_at: string | null
			assets: { name: string; browser_download_url: string }[]
		}[]
		// The list has no guaranteed order, so take the newest published release with the asset.
		let best: Release | undefined
		for (const r of releases) {
			const asset = r.assets.find((a) => a.name === ASSET)
			if (r.draft || !asset || !r.published_at || !wantedTag(r.tag_name)) continue
			if (!best || Date.parse(r.published_at) > Date.parse(best.publishedAt)) {
				best = { tag: r.tag_name, publishedAt: r.published_at, url: asset.browser_download_url }
			}
		}
		return best
	}

	/**
	 * The releases Atom feed: not subject to the API's rate limit, dated, but without assets, so
	 * each candidate's download URL is checked newest first.
	 */
	async function newestFromFeed(): Promise<Release | undefined> {
		const response = await fetch(`https://github.com/${GITHUB_REPO}/releases.atom`, {
			headers: { 'User-Agent': 'amp-prologue-plugin' },
			signal: AbortSignal.timeout(20_000),
		})
		if (!response.ok) throw new Error(`releases feed returned HTTP ${response.status}`)
		const xml = await response.text()
		const entries: Release[] = []
		for (const [, entry] of xml.matchAll(/<entry>([\s\S]*?)<\/entry>/g)) {
			const tag = entry!.match(/\/releases\/tag\/([^"<]+)"/)?.[1]
			const updated = entry!.match(/<updated>([^<]+)<\/updated>/)?.[1]
			const decoded = tag && decodeURIComponent(tag)
			if (decoded && wantedTag(decoded) && updated && !Number.isNaN(Date.parse(updated))) {
				entries.push({ tag: decoded, publishedAt: updated, url: assetURL(decoded) })
			}
		}
		if (!entries.length) throw new Error('releases feed had no entries')
		entries.sort((a, b) => Date.parse(b.publishedAt) - Date.parse(a.publishedAt))
		for (const candidate of entries.slice(0, 10)) {
			const head = await fetch(candidate.url, { method: 'HEAD', redirect: 'manual', signal: AbortSignal.timeout(15_000) })
			if (head.status >= 200 && head.status < 400) return candidate
		}
		return undefined
	}

	/** The newest web release, trying sources until one answers. `undefined` means none has the asset. */
	async function lookupRelease(): Promise<{ release?: Release; errors: string[] }> {
		const errors: string[] = []
		const token = await githubToken().catch(() => undefined)
		const sources: [string, () => Promise<Release | undefined>][] = [
			...(token ? [['GitHub API (token)', () => newestFromAPI(token)] as [string, () => Promise<Release | undefined>]] : []),
			['releases feed', newestFromFeed],
			['GitHub API', () => newestFromAPI()],
		]
		for (const [name, source] of sources) {
			try {
				const release = await source()
				log(`lookup via ${name}: ${release?.tag ?? 'no release with the web asset'}`)
				return { release, errors }
			} catch (error) {
				errors.push(`${name}: ${error instanceof Error ? error.message : error}`)
			}
		}
		return { errors }
	}

	/** `notice` is set when the lookup failed and an older cached build is used instead. */
	type BundleResult = { bundle: Bundle; downloaded: boolean; notice?: string } | { error: string }

	/** One release lookup and download at a time, shared by prefetch, commands, and the tool. */
	interface BundleJob {
		promise: Promise<BundleResult>
		/** The tag being downloaded, once a download (not just a lookup) is under way. */
		downloading?: string
		onDownload: Set<(tag: string) => void>
	}
	let job: BundleJob | undefined

	function bundleJob(): BundleJob {
		if (job) return job
		const current: BundleJob = { promise: Promise.resolve({ error: '' }), onDownload: new Set() }
		current.promise = resolveBundle(current)
			.catch((error): BundleResult => ({ error: `Couldn’t get Prologue: ${error instanceof Error ? error.message : error}` }))
			.then(async (result) => {
				if ('bundle' in result) await linkCli(result.bundle.cli).catch(() => {})
				return result
			})
			.finally(() => {
				if (job === current) job = undefined
			})
		job = current
		return current
	}

	/** Find the newest release with the web asset and make sure it is extracted in the cache. Never notifies. */
	async function resolveBundle(current: BundleJob): Promise<BundleResult> {
		if (process.platform !== 'linux' || process.arch !== 'x64') {
			return { error: `Prologue’s web release is built for x86_64 Linux only, not ${process.platform}/${process.arch}.` }
		}

		const { release, errors } = await lookupRelease()
		if (!release) {
			const cached = await newestCachedBundle()
			if (errors.length) {
				const why = errors.join('; ')
				log(`lookup failed (${why})`)
				if (cached) {
					return {
						bundle: cached,
						downloaded: false,
						notice: `Couldn’t check GitHub for a newer Prologue build (${why}), so this uses the cached build ${cached.label}. It may be out of date.`,
					}
				}
				return { error: `Couldn’t check github.com/${GITHUB_REPO} for Prologue’s web release (${why}), and none is cached yet. Try again in a minute.` }
			}
			if (cached) return { bundle: cached, downloaded: false }
			return { error: `Prologue’s web release isn’t published yet: no release on github.com/${GITHUB_REPO} has ${ASSET}. Start Review will work once it is.` }
		}

		const dir = join(cacheDir, 'releases', release.tag)
		const meta = JSON.stringify({ tag: release.tag, publishedAt: release.publishedAt })
		const cached = await findBundle(dir, release.tag)
		if (cached) {
			// Older caches have no meta.json; record the date so ordering stays right.
			if (!existsSync(join(dir, 'meta.json'))) await writeFile(join(dir, 'meta.json'), meta).catch(() => {})
			return { bundle: cached, downloaded: false }
		}

		current.downloading = release.tag
		for (const listener of current.onDownload) listener(release.tag)
		log(`downloading ${release.tag} (published ${release.publishedAt})`)
		const staging = `${dir}.download-${process.pid}`
		await rm(staging, { recursive: true, force: true })
		await mkdir(staging, { recursive: true })
		try {
			const response = await fetch(release.url, { signal: AbortSignal.timeout(180_000) })
			if (!response.ok) throw new Error(`download failed with HTTP ${response.status}`)
			const bytes = new Uint8Array(await response.arrayBuffer())
			// Releases publish `sha256sum` output next to the archive; refuse a build without one.
			const sums = await fetch(`${release.url}.sha256`, { signal: AbortSignal.timeout(20_000) })
			if (!sums.ok) throw new Error(`no ${ASSET}.sha256 in the release (HTTP ${sums.status})`)
			const expected = (await sums.text()).trim().split(/\s+/)[0]?.toLowerCase()
			const actual = createHash('sha256').update(bytes).digest('hex')
			if (expected !== actual) throw new Error(`checksum mismatch: expected ${expected}, got ${actual}`)
			const archive = join(staging, ASSET)
			await writeFile(archive, bytes)
			const untar = await run('tar', ['-xzf', archive, '-C', staging])
			if (untar.code !== 0) throw new Error(`tar failed: ${untar.stderr.trim()}`)
			await rm(archive)
			if (!(await findBundle(staging, release.tag))) {
				throw new Error(`${ASSET} doesn’t contain prologue-server, prologue, and dist/`)
			}
			await writeFile(join(staging, 'meta.json'), meta)
			await rm(dir, { recursive: true, force: true })
			// Older releases stay: a running server reads its dist/ on every page load, and the cache
			// goes away with the orb.
			await rename(staging, dir)
		} catch (error) {
			await rm(staging, { recursive: true, force: true })
			const why = `Couldn’t download Prologue ${release.tag}: ${error instanceof Error ? error.message : error}`
			const fallback = await newestCachedBundle()
			if (fallback) return { bundle: fallback, downloaded: false, notice: `${why}. Using the cached build ${fallback.label} instead.` }
			return { error: why }
		}
		const bundle = await findBundle(dir, release.tag)
		log(`downloaded ${release.tag}`)
		return bundle ? { bundle, downloaded: true } : { error: `Prologue ${release.tag} is missing files after extracting.` }
	}

	/**
	 * The bundle for someone waiting on it: always checks for a newer build (joining a lookup or
	 * download already running), and says when a download starts, finishes, or fails, and when an
	 * older cached build stands in.
	 */
	async function getBundle(ui: PluginUI): Promise<{ bundle?: Bundle; notice?: string }> {
		const current = bundleJob()
		let announced: Promise<void> | undefined
		const announce = (tag: string) => {
			announced ??= ui.notify(`Downloading Prologue ${tag}… The review opens when it’s ready.`).catch(() => {})
		}
		if (current.downloading) announce(current.downloading)
		else current.onDownload.add(announce)
		const result = await current.promise
		current.onDownload.delete(announce)
		await announced
		if ('error' in result) {
			await ui.notify(result.error)
			return {}
		}
		if (result.notice) await ui.notify(result.notice)
		else if (announced) await ui.notify(`Prologue ${result.bundle.label} downloaded.`)
		return { bundle: result.bundle, notice: result.notice }
	}

	/**
	 * Move a running server onto a newer bundle with the same repo, base, and webhook. The portal
	 * URL stays the same; open pages reconnect and offer to reload into the new build.
	 */
	let swapping = false
	async function swapToBundle(bundle: Bundle): Promise<void> {
		const previous = (await loadSettings()).service
		if (swapping || !previous || previous.server === bundle.server || !(await serviceRunning())) return
		swapping = true
		try {
			log(`swapping the running server to ${bundle.label}`)
			const url = await startServer(undefined, previous.repo, previous.base, bundle)
			log(url ? `swapped to ${bundle.label}` : `swap to ${bundle.label} failed`)
		} finally {
			swapping = false
		}
	}

	/** Look for a newer build in the background, at most every few minutes, so a click rarely waits. */
	const PREFETCH_INTERVAL_MS = 3 * 60_000
	let lastPrefetch = 0
	function prefetch(): void {
		if (Date.now() - lastPrefetch < PREFETCH_INTERVAL_MS) return
		lastPrefetch = Date.now()
		void bundleJob().promise.then((result) => {
			if ('error' in result) log(`prefetch failed: ${result.error}`)
			else {
				log(`prefetch ready: ${result.bundle.label}${result.notice ? ` (${result.notice})` : ''}`)
				// A notice means the lookup failed and this may not be the newest build.
				if (!result.notice) return swapToBundle(result.bundle).catch((error) => log(`swap failed: ${error}`))
			}
		})
	}

	/** Put the CLI on PATH (~/.local/bin) so the agent can run `prologue`. Never replaces a real file. */
	async function linkCli(cli: string): Promise<void> {
		const binDir = join(homedir(), '.local', 'bin')
		const link = join(binDir, 'prologue')
		await mkdir(binDir, { recursive: true })
		const existing = await lstat(link).catch(() => undefined)
		if (existing && !existing.isSymbolicLink()) return
		if (existing) await rm(link)
		await symlink(cli, link)
	}

	// ── Repository and base ref ──

	async function gitTop(dir: string): Promise<string | undefined> {
		const top = await git(dir, 'rev-parse', '--show-toplevel')
		return top.code === 0 ? top.stdout.trim() : undefined
	}

	async function findRepo(ui: PluginUI): Promise<string | undefined> {
		const top = await gitTop(workspaceRoot)
		if (top) return top
		// Threads without a project: offer the git checkouts directly under the workspace.
		const repos: string[] = []
		for (const entry of await readdir(workspaceRoot, { withFileTypes: true }).catch(() => [])) {
			if (entry.isDirectory() && !entry.name.startsWith('.') && existsSync(join(workspaceRoot, entry.name, '.git'))) {
				repos.push(join(workspaceRoot, entry.name))
			}
		}
		if (repos.length === 1) return repos[0]
		if (repos.length === 0) {
			await ui.notify(`No git repository found in ${workspaceRoot}.`)
			return undefined
		}
		return ui.select({ title: 'Prologue: Which repository?', message: 'Review the changes in:', options: repos })
	}

	/** `owner/name` from the origin remote, or the path for repos without one. */
	async function projectKey(repo: string): Promise<string> {
		const origin = (await git(repo, 'remote', 'get-url', 'origin')).stdout.trim()
		const match = origin.match(/[:/]([^/:]+\/[^/]+?)(?:\.git)?\/?$/)
		return match ? match[1]!.toLowerCase() : repo
	}

	async function suggestBase(repo: string): Promise<string> {
		if (DEFAULT_BASE) return DEFAULT_BASE
		const head = await git(repo, 'symbolic-ref', '--short', 'refs/remotes/origin/HEAD')
		if (head.code === 0 && head.stdout.trim()) return head.stdout.trim()
		for (const candidate of ['origin/main', 'origin/master', 'main', 'master']) {
			if ((await git(repo, 'rev-parse', '--verify', '--quiet', `${candidate}^{commit}`)).code === 0) return candidate
		}
		return 'origin/main'
	}

	async function askBase(ui: PluginUI, repo: string, key: string, current?: string): Promise<string | undefined> {
		const base = await ui.input({
			title: `Prologue: Base for ${key}`,
			helpText: 'Prologue reviews the changes on this branch since this ref. It is remembered for this project.',
			initialValue: current ?? (await suggestBase(repo)),
			submitButtonText: 'Review',
		})
		return base?.trim() || undefined
	}

	/**
	 * Make sure `base` resolves and shares history with HEAD: orb checkouts are often shallow and
	 * only have the default branch.
	 */
	async function prepareBase(ui: PluginUI, repo: string, base: string): Promise<boolean> {
		const resolves = async () => (await git(repo, 'rev-parse', '--verify', '--quiet', `${base}^{commit}`)).code === 0
		if (!(await resolves())) {
			const remotes = (await git(repo, 'remote')).stdout.split('\n').filter(Boolean)
			const remote = remotes.find((r) => base.startsWith(`${r}/`))
			if (remote) {
				const branch = base.slice(remote.length + 1)
				await ui.notify(`Fetching ${base}…`)
				await git(repo, 'fetch', '--quiet', remote, `+refs/heads/${branch}:refs/remotes/${remote}/${branch}`)
			}
		}
		if (!(await resolves())) {
			await ui.notify(`Base ref ${base} doesn’t exist in ${repo}. Run “Prologue: Set Base Ref” to change it.`)
			return false
		}
		if ((await git(repo, 'merge-base', 'HEAD', base)).code !== 0) {
			const shallow = (await git(repo, 'rev-parse', '--is-shallow-repository')).stdout.trim() === 'true'
			if (shallow) {
				await ui.notify('Fetching full history so Prologue can find where this branch started…')
				await run('git', ['-C', repo, 'fetch', '--quiet', '--unshallow'], undefined, 600_000)
			}
		}
		return true
	}

	// ── Service ──

	// Orb services belong to the folder they were started from, so every call runs there.
	const service = (verb: string, ...args: string[]) =>
		run('amp', ['orb', 'service', verb, SERVICE, ...args], workspaceRoot, 120_000)

	async function serviceRunning(): Promise<boolean> {
		const status = await service('status')
		return /Active:\s+active \(running\)/.test(status.stdout)
	}

	async function portalURL(): Promise<string | undefined> {
		try {
			const manifest = JSON.parse(await readFile(join(workspaceRoot, '.amp', 'portals', `${SERVICE}.json`), 'utf8'))
			return manifest.links?.[0]?.url
		} catch {
			return undefined
		}
	}

	/** Keep the generated portal manifest out of the checkout’s Changes. */
	async function excludePortalManifest(): Promise<void> {
		const top = await gitTop(workspaceRoot)
		if (!top) return
		const ignored = await git(top, 'check-ignore', '-q', join(workspaceRoot, '.amp', 'portals', `${SERVICE}.json`))
		if (ignored.code === 0) return
		const exclude = (await git(top, 'rev-parse', '--git-path', 'info/exclude')).stdout.trim()
		if (!exclude) return
		const path = exclude.startsWith('/') ? exclude : join(top, exclude)
		const current = await readFile(path, 'utf8').catch(() => '')
		await mkdir(join(path, '..'), { recursive: true })
		await writeFile(path, `${current}${current && !current.endsWith('\n') ? '\n' : ''}.amp/portals/\n`)
	}

	let webhookURL: Promise<string | undefined> = Promise.resolve(undefined)

	/** Without `ui` (a background swap), progress and failures go to the log only. */
	async function startServer(ui: PluginUI | undefined, repo: string, base: string, bundle: Bundle): Promise<string | undefined> {
		const webhook = (await webhookURL) ?? ''
		// The thread owner: Prologue lets them edit and delete anyone's comments. Older releases ignore it.
		const ownerId = amp.system.user?.id ?? ''
		const ownerEmail = amp.system.user?.email ?? ''
		const settings = await loadSettings()
		const previous = settings.service
		if (await serviceRunning()) {
			if (
				previous &&
				previous.repo === repo &&
				previous.base === base &&
				previous.server === bundle.server &&
				previous.webhook === webhook &&
				(previous.ownerId ?? '') === ownerId &&
				(previous.ownerEmail ?? '') === ownerEmail &&
				previous.pluginVersion === PLUGIN_VERSION
			) {
				const url = (await portalURL()) ?? previous.url
				if (url) return url
			}
			// `service start` with an unchanged command keeps the old process, and the launcher
			// script's path never changes, so stop it before starting with the new settings.
			await service('stop')
		}

		// The webhook URL is a credential, so it goes in a private launcher script instead of the
		// service command line.
		const script = join(cacheDir, 'run-prologue-server.sh')
		const lines = [
			'#!/bin/sh',
			`export PROLOGUE_REPO=${shellQuote(repo)}`,
			`export PROLOGUE_BASE=${shellQuote(base)}`,
			webhook ? `export PROLOGUE_AMP_WEBHOOK_URL=${shellQuote(webhook)}` : 'unset PROLOGUE_AMP_WEBHOOK_URL',
			ownerId ? `export PROLOGUE_OWNER_ID=${shellQuote(ownerId)}` : 'unset PROLOGUE_OWNER_ID',
			ownerEmail ? `export PROLOGUE_OWNER_EMAIL=${shellQuote(ownerEmail)}` : 'unset PROLOGUE_OWNER_EMAIL',
			`export PROLOGUE_PLUGIN_VERSION=${PLUGIN_VERSION}`,
			// Loopback only: the server has no authentication of its own, and the portal reaches it there.
			`exec ${shellQuote(bundle.server)} --host 127.0.0.1 --dist ${shellQuote(bundle.dist)}`,
			'',
		]
		await mkdir(cacheDir, { recursive: true })
		await writeFile(script, lines.join('\n'), { mode: 0o700 })
		await chmod(script, 0o700)

		await ui?.notify(`Starting Prologue (${bundle.label}) for ${repo} against ${base}…`)
		const started = await service(
			'start',
			'--command', `exec ${shellQuote(script)}`,
			'--cwd', repo,
			'--portal',
			'--title', 'Prologue',
			'--description', `Review of ${repo} against ${base}`,
		)
		const url = started.stdout.match(/https?:\/\/\S+/g)?.pop()
		if (started.code !== 0 || !url) {
			const logs = await service('logs')
			const tail = (logs.stdout || started.stderr || started.stdout).trim().split('\n').slice(-5).join('\n')
			if (ui) await ui.notify(`Prologue didn’t start.\n${tail}`)
			else log(`Prologue didn’t start: ${tail}`)
			return undefined
		}
		await excludePortalManifest().catch(() => {})
		await updateSettings((s) => {
			s.service = { repo, base, server: bundle.server, webhook, ownerId, ownerEmail, pluginVersion: PLUGIN_VERSION, url }
		})
		return url
	}

	async function openPortal(ui: PluginUI, url: string): Promise<void> {
		try {
			await ui.perform({ action: 'open-portal', args: { url } })
			log(`opened portal ${url}`)
		} catch (error) {
			log(`open-portal failed: ${error}`)
			await ui.notify(`Prologue is running at ${url}, but the Portal pane didn’t open. Open it from the Portal tab.`)
		}
	}

	/** Start (or reuse) the Prologue service and open it in the Portal pane. Returns a status line. */
	async function startReview(ui: PluginUI, options: { base?: string; askBase?: boolean } = {}): Promise<string> {
		const repo = await findRepo(ui)
		if (!repo) return 'No repository selected.'
		const key = await projectKey(repo)
		const remembered = (await loadSettings()).bases[key]
		let base = options.base?.trim() || (options.askBase ? undefined : remembered)
		// The workspace default isn't remembered, so a later change to it still applies.
		const fromDefault = !base && !options.askBase && !!DEFAULT_BASE
		if (fromDefault) base = DEFAULT_BASE
		if (!base) {
			base = await askBase(ui, repo, key, remembered)
			if (!base) return 'Cancelled: no base ref.'
		}
		if (!fromDefault && base !== remembered) {
			const chosen = base
			await updateSettings((s) => {
				s.bases[key] = chosen
			})
		}

		const { bundle, notice } = await getBundle(ui)
		if (!bundle) return 'Prologue’s web bundle isn’t available (see the notification).'
		if (!(await prepareBase(ui, repo, base))) return `Base ref ${base} is not available.`

		const url = await startServer(ui, repo, base, bundle)
		if (!url) return 'Prologue failed to start; see `amp orb service logs prologue`.'
		await openPortal(ui, url)
		return `${notice ? `${notice}\n` : ''}Prologue ${bundle.label} is reviewing ${repo} against ${base} at ${url} (opened in the Portal pane).`
	}

	// ── Commands, quick action, and a tool for the agent ──

	amp.registerCommand(
		START_COMMAND,
		{
			title: 'Start Review',
			category: 'Prologue',
			description: 'Open Prologue in the Portal pane to review this orb’s changes.',
		},
		async (ctx) => {
			await startReview(ctx.ui)
		},
	)

	amp.registerCommand(
		'set-base',
		{
			title: 'Set Base Ref',
			category: 'Prologue',
			description: 'Change the base ref Prologue reviews against for this project, then reopen it.',
		},
		async (ctx) => {
			await startReview(ctx.ui, { askBase: true })
		},
	)

	amp.registerTool({
		name: 'prologue_start_review',
		title: 'Start Prologue review',
		description:
			'Start (or reuse) the Prologue code review web UI for this orb’s repository and open it in the user’s Portal pane. Use when the user asks to review changes in Prologue.',
		inputSchema: {
			type: 'object',
			properties: {
				base: {
					type: 'string',
					description:
						'Base ref to review against, such as origin/main. Omit to use the remembered base for this project, else PROLOGUE_DEFAULT_BASE (otherwise the user is asked the first time).',
				},
			},
		},
		async execute(input, ctx) {
			return startReview(ctx.ui, { base: typeof input.base === 'string' ? input.base : undefined })
		},
	})

	// ── Start button: shown only while the checkout has changes against its base ──

	/** The checkout, or for threads without a project, the git repos directly under the workspace. */
	async function candidateRepos(): Promise<string[]> {
		const top = await gitTop(workspaceRoot)
		if (top) return [top]
		const repos: string[] = []
		for (const entry of await readdir(workspaceRoot, { withFileTypes: true }).catch(() => [])) {
			if (entry.isDirectory() && !entry.name.startsWith('.') && existsSync(join(workspaceRoot, entry.name, '.git'))) {
				repos.push(join(workspaceRoot, entry.name))
			}
		}
		return repos
	}

	/**
	 * The remembered base, else PROLOGUE_DEFAULT_BASE, else origin's default branch. Local refs only;
	 * never fetches or asks, so a base that isn't fetched yet (shallow orb checkouts often lack
	 * origin/production) falls through to the next candidate until Start Review fetches it.
	 */
	async function checkBase(repo: string): Promise<string | undefined> {
		const remembered = (await loadSettings()).bases[await projectKey(repo)]
		const head = (await git(repo, 'symbolic-ref', '--short', 'refs/remotes/origin/HEAD')).stdout.trim()
		for (const candidate of [remembered, DEFAULT_BASE, head, 'origin/main', 'origin/master']) {
			if (candidate && (await git(repo, 'rev-parse', '--verify', '--quiet', `${candidate}^{commit}`)).code === 0) {
				return candidate
			}
		}
		return undefined
	}

	/** Staged, unstaged, or untracked changes (outside .amp/), or commits ahead of the base. */
	async function hasChanges(repo: string): Promise<boolean> {
		const status = await git(repo, 'status', '--porcelain', '--untracked-files=normal')
		if (status.code !== 0) throw new Error(status.stderr.trim())
		if (status.stdout.split('\n').some((line) => line.length > 3 && !line.slice(3).replace(/^"/, '').startsWith('.amp/'))) {
			return true
		}
		const base = await checkBase(repo)
		if (!base) return false
		const ahead = await git(repo, 'rev-list', '--count', `${base}..HEAD`)
		return ahead.code === 0 && Number(ahead.stdout.trim()) > 0
	}

	const startActions = new Map<ThreadID, QuickAction | undefined>()
	let checking: Promise<void> | undefined

	function refreshStartActions(): Promise<void> {
		checking ??= (async () => {
			let changed = false
			try {
				for (const repo of await candidateRepos()) {
					if (await hasChanges(repo)) {
						changed = true
						break
					}
				}
			} catch (error) {
				log(`change check failed: ${error}`)
				return
			}
			for (const [id, action] of startActions) {
				if (changed && !action) {
					try {
						startActions.set(
							id,
							amp.threads.get(id).createQuickAction({
								text: 'Prologue Review',
								icon: 'eye',
								action: 'run-command',
								args: { command: START_COMMAND },
							}),
						)
						log(`changes found: showing Prologue Review in ${id}`)
					} catch (error) {
						log(`couldn’t show the quick action: ${error}`)
					}
				} else if (!changed && action) {
					action.unsubscribe()
					startActions.set(id, undefined)
					log(`no changes: hid Prologue Review in ${id}`)
				}
			}
			if (changed) prefetch()
		})().finally(() => {
			checking = undefined
		})
		return checking
	}

	function watchThread(id: ThreadID): void {
		if (!startActions.has(id)) startActions.set(id, undefined)
		void refreshStartActions()
	}

	amp.on('session.start', (event) => watchThread(event.thread.id))
	amp.on('agent.end', () => {
		void refreshStartActions()
	})
	// session.start doesn't fire again when the plugin reloads in an open thread.
	if (process.env.AMP_THREAD_ID?.startsWith('T-')) watchThread(process.env.AMP_THREAD_ID as ThreadID)
	// Edits made outside the agent's turns, such as in the Terminal tab: a cheap git poll.
	const poll = setInterval(() => {
		if (startActions.size) void refreshStartActions()
	}, 30_000)
	amp.onDispose(() => clearInterval(poll))

	// ── Send to Amp: webhook → queued user message, plus a pending quick action ──

	const pending = new Map<ThreadID, { ids: Set<number>; repo?: string; action?: QuickAction }>()

	function showPending(thread: PluginThread): void {
		const entry = pending.get(thread.id)
		if (!entry) return
		if (entry.ids.size === 0) {
			entry.action?.unsubscribe()
			pending.delete(thread.id)
			return
		}
		const n = entry.ids.size
		const definition = {
			text: `Process ${n} review comment${n === 1 ? '' : 's'}`,
			icon: 'message-square' as const,
			action: 'send-message' as const,
			args: { message: reviewMessage([...entry.ids], n, entry.repo) },
		}
		if (entry.action) entry.action.update(definition)
		else entry.action = thread.createQuickAction(definition)
	}

	/** Drop sent comments that are closed or already have a reply after the reviewer’s last word. */
	async function refreshPending(thread: PluginThread): Promise<void> {
		const entry = pending.get(thread.id)
		const repo = entry?.repo ?? (await loadSettings()).service?.repo
		if (!entry || !repo) return
		const shown = await run('prologue', ['show', '--json'], repo, 30_000)
		if (shown.code !== 0) return
		try {
			const data = JSON.parse(shown.stdout) as {
				threads: { root: { id: number; state: string; author: string }; replies: { author: string }[] }[]
			}
			const waiting = new Set<number>()
			for (const t of data.threads) {
				const last = t.replies.at(-1) ?? t.root
				if (t.root.state === 'open' && last.author === 'reviewer') waiting.add(t.root.id)
			}
			for (const id of entry.ids) if (!waiting.has(id)) entry.ids.delete(id)
			showPending(thread)
		} catch {
			// Unknown output: leave the button as it is.
		}
	}

	/** The bundled CLI next to the running server, else `prologue` on PATH (linked to the bundle). */
	async function cliPath(): Promise<string> {
		const server = (await loadSettings()).service?.server
		const bundled = server ? join(dirname(server), 'prologue') : undefined
		return bundled && existsSync(bundled) ? bundled : 'prologue'
	}

	/** Tell Prologue the message landed, so it stops reverting the sent comments. */
	async function confirmSend(sendId: string, repo: string): Promise<string> {
		const result = await run(await cliPath(), ['confirm-send', sendId], repo, 15_000)
		const output = (result.stdout + result.stderr).trim().replace(/\s+/g, ' ').slice(0, 300)
		return `exit ${result.code}${output ? `: ${output}` : ''}`
	}

	const inFlight = new Set<string>()
	// Project threads owned by the same user share a registration per key, and its handler runs in
	// whichever thread owns it, so each thread needs its own key: otherwise a send from a second
	// thread is appended to, and confirmed in, the first thread's orb.
	const threadKey = process.env.AMP_THREAD_ID?.startsWith('T-') ? `-${process.env.AMP_THREAD_ID}` : ''
	webhookURL = amp
		.createWebhook({
			key: `send-to-amp${threadKey}`,
			handler: async (event, ctx) => {
				if (inFlight.has(event.id)) return
				inFlight.add(event.id)
				try {
					let payload: { type?: unknown; threadID?: unknown; commentIds?: unknown; count?: unknown; repo?: unknown; sendId?: unknown }
					try {
						payload = JSON.parse(new TextDecoder().decode(event.body))
					} catch {
						log(`webhook ${event.id}: ignored, body is not JSON`)
						return
					}
					// Anyone holding the webhook URL controls the payload, so its thread must be one this
					// orb hosts.
					const payloadThread =
						typeof payload.threadID === 'string' && startActions.has(payload.threadID as ThreadID)
							? (payload.threadID as ThreadID)
							: ctx.thread.id

					// The update banner's button: the message is this plugin's own text, never the payload's.
					if (payload.type === 'update-plugin') {
						if ((await loadSettings()).handledEvents.includes(event.id)) return
						const thread = payloadThread === ctx.thread.id ? ctx.thread : amp.threads.get(payloadThread)
						await thread.appendUserMessage({ type: 'user-message', content: PLUGIN_UPDATE_MESSAGE })
						log(`webhook ${event.id}: update request appended to ${payloadThread}`)
						await updateSettings((s) => {
							s.handledEvents = [...s.handledEvents, event.id].slice(-200)
						})
						return
					}
					const ids = (Array.isArray(payload.commentIds) ? payload.commentIds : [])
						.map((id) => Number(String(id).replace(/^C/i, '')))
						.filter((id) => Number.isInteger(id) && id > 0)
					const count = ids.length || Math.max(0, Math.floor(Number(payload.count) || 0))
					// Older Prologue releases send no sendId; those sends need no confirmation.
					const sendId =
						typeof payload.sendId === 'string' && /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(payload.sendId)
							? payload.sendId
							: undefined
					// Likewise, its repository must be a checkout in this workspace.
					const claimedRepo = typeof payload.repo === 'string' ? resolve(payload.repo) : undefined
					const payloadRepo =
						claimedRepo &&
						(claimedRepo === workspaceRoot || claimedRepo.startsWith(`${workspaceRoot}/`)) &&
						existsSync(join(claimedRepo, '.git'))
							? claimedRepo
							: undefined
					const repo = payloadRepo ?? (await loadSettings()).service?.repo
					const what = `webhook ${event.id}: sendId ${sendId ?? 'none'}, comments ${ids.length ? formatCommentIds(ids) : `${count} (no ids)`}`

					const redelivered = (await loadSettings()).handledEvents.includes(event.id)
					if (redelivered) {
						log(`${what}: already delivered, not appending again`)
					} else {
						if (count > 0) {
							const threadID = payloadThread
							const thread = threadID === ctx.thread.id ? ctx.thread : amp.threads.get(threadID)
							const repoNote = payloadRepo && payloadRepo !== workspaceRoot ? payloadRepo : undefined
							try {
								// No steer: if the agent is mid-turn, this waits in the queue.
								await thread.appendUserMessage({ type: 'user-message', content: reviewMessage(ids, count, repoNote) })
							} catch (error) {
								// Unconfirmed, Prologue reverts the comments after 60 s so they can be sent again.
								log(`${what}: appending to ${threadID} failed, not confirming: ${error instanceof Error ? error.message : error}`)
								return
							}
							log(`${what}: appended to ${threadID}`)

							const entry = pending.get(thread.id) ?? { ids: new Set<number>() }
							entry.repo = payloadRepo ?? entry.repo
							for (const id of ids) entry.ids.add(id)
							pending.set(thread.id, entry)
							try {
								showPending(thread)
							} catch (error) {
								log(`couldn’t show the pending quick action: ${error}`)
							}
						}
						// Recorded before confirming, so a redelivery after a failed confirm doesn't append twice.
						await updateSettings((s) => {
							s.handledEvents = [...s.handledEvents, event.id].slice(-200)
						})
					}

					if (sendId) {
						if (!repo) {
							log(`${what}: can’t confirm, no review repository known`)
						} else {
							// confirm-send is idempotent, so a redelivery confirms again.
							log(`${what}: confirm-send in ${repo}: ${await confirmSend(sendId, repo)}`)
						}
					}
				} finally {
					inFlight.delete(event.id)
				}
			},
		})
		.then((registration) => registration.url)
		.catch((error) => {
			amp.logger.log(`Prologue webhook unavailable; Send to Amp is disabled: ${error}`)
			return undefined
		})

	amp.on('agent.end', async (event, ctx) => {
		if (pending.has(event.thread.id)) await refreshPending(ctx.thread)
	})
}
