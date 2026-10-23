import { createRouter, createWebHistory, type RouteRecordNameGeneric, type RouterHistory } from 'vue-router'
import type { SandpostClient } from '@/api/contracts'

/**
 * Routes inside the authenticated mail interface.
 */
const MAIL_ROUTES = new Set<RouteRecordNameGeneric>(['mail', 'mail-message'])

/**
 * Routes inside the administration area, whose layout verifies the session itself.
 */
const ADMIN_ROUTES = new Set<RouteRecordNameGeneric>(['admin-scopes', 'admin-smtp-access', 'admin-smtp-outbound', 'admin-users'])

/**
 * The signed-in user's IMAP account pages, which handle session errors themselves.
 */
const IMAP_ROUTES = new Set<RouteRecordNameGeneric>(['imap', 'imap-account'])

/**
 * Create the SPA router and enforce instance setup followed by owner login.
 */
export function createAppRouter(client: SandpostClient, history: RouterHistory = createWebHistory()) {
  const router = createRouter({
    history,
    routes: [
      { path: '/', redirect: { name: 'mail' } },
      {
        path: '/setup',
        name: 'setup',

        /**
         * Load the owner setup page when its route is entered.
         */
        component: () => import('@/features/setup/SetupPage.vue')
      },
      {
        path: '/login',
        name: 'login',

        /**
         * Load the login page when its route is entered.
         */
        component: () => import('@/features/auth/LoginPage.vue')
      },
      {
        /*
         * Mail and administration share one shell, so its sidebar stays mounted while navigating between them.
         */
        path: '/',

        /**
         * Load the persistent shell shared by authenticated routes.
         */
        component: () => import('@/layouts/AppShell.vue'),
        children: [
          {
            path: 'mail',
            name: 'mail',

            /**
             * Load the mail page for the current folder or search.
             */
            component: () => import('@/features/mail/MailPage.vue')
          },
          {
            path: 'mail/:id',
            name: 'mail-message',

            /**
             * Load the same mail page with a selected message identifier.
             */
            component: () => import('@/features/mail/MailPage.vue')
          },
          /*
           * IMAP accounts belong to every signed-in user, whatever their role.
           */
          {
            path: 'imap',
            name: 'imap',

            /**
             * Load the signed-in user's IMAP account list.
             */
            component: () => import('@/features/imap/ImapAccountsPage.vue')
          },
          {
            path: 'imap/:id',
            name: 'imap-account',

            /**
             * Load the editor for the selected IMAP account.
             */
            component: () => import('@/features/imap/ImapAccountPage.vue')
          },
          {
            path: 'admin',

            /**
             * Load the layout that checks access to administration pages.
             */
            component: () => import('@/layouts/AdminLayout.vue'),
            children: [
              { path: '', name: 'admin', redirect: { name: 'admin-scopes' } },
              {
                path: 'scopes',
                name: 'admin-scopes',

                /**
                 * Load administration of scope definitions and assignments.
                 */
                component: () => import('@/features/admin/ScopesPage.vue')
              },
              {
                path: 'smtp-access',
                name: 'admin-smtp-access',

                /**
                 * Load administration of the SMTP listener and submission accesses.
                 */
                component: () => import('@/features/admin/SmtpAccessPage.vue')
              },
              {
                path: 'smtp-outbound',
                name: 'admin-smtp-outbound',

                /**
                 * Load administration of SMTP Outbound connections and recent deliveries.
                 */
                component: () => import('@/features/admin/SmtpOutboundPage.vue')
              },
              {
                path: 'users',
                name: 'admin-users',

                /**
                 * Load administration of users and their access policies.
                 */
                component: () => import('@/features/admin/UsersPage.vue')
              },
            ],
          },
        ],
      },
    ],
  })

  router.beforeEach(async (to, from) => {
    /*
     * Selecting a message only changes the route parameter; the mail page handles API session errors itself.
     */
    if (MAIL_ROUTES.has(from.name) && MAIL_ROUTES.has(to.name)) return true

    /*
     * Moving between admin pages keeps the layout mounted; it handles session errors itself.
     */
    if (ADMIN_ROUTES.has(from.name) && ADMIN_ROUTES.has(to.name)) return true
    if (IMAP_ROUTES.has(from.name) && IMAP_ROUTES.has(to.name)) return true

    let initialized: boolean
    try {
      ({ initialized } = await client.instanceStatus())
    } catch {
      return to.name === 'setup' || to.name === 'login' ? true : { name: 'login' }
    }
    if (!initialized) return to.name === 'setup' ? true : { name: 'setup' }

    let user
    try {
      user = await client.session()
    } catch {
      user = null
    }
    if (to.name === 'setup') return user ? { name: 'mail' } : { name: 'login' }
    if (to.name === 'login') return user ? { name: 'mail' } : true
    return user ? true : { name: 'login' }
  })

  return router
}
