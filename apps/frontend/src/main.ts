import { createApp } from 'vue'
import App from './App.vue'
import { createSandpostClient, sandpostClientKey } from '@/runtime/client'
import { createAppRouter } from '@/router'
import { insetTitleBar } from '@/runtime/windowChrome'
import { i18n, syncLocale } from '@/i18n'
import './style.css'

/*
 * Let the shell reserve room for, and drag the window around, macOS traffic lights.
 */
if (insetTitleBar) document.documentElement.dataset.titleBar = 'inset'

const client = createSandpostClient()
syncLocale()
createApp(App)
  .use(i18n)
  .provide(sandpostClientKey, client)
  .use(createAppRouter(client))
  .mount('#app')
