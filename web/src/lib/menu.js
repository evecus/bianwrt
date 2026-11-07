import { h } from 'vue'
import { NIcon } from 'naive-ui'
import {
  StatsChart, AnalyticsOutline, ThermometerOutline, ReaderOutline,
  PeopleOutline, PulseOutline, GitNetwork, WifiOutline,
  LayersOutline, GitCompareOutline, BarChartOutline, SpeedometerOutline,
  ShieldOutline, PlanetOutline, FunnelOutline, KeyOutline,
  SettingsOutline, ServerOutline, CalendarOutline, ArchiveOutline,
  PricetagsOutline, CloudOutline, RocketOutline, ExtensionPuzzleOutline,
  HomeOutline, HardwareChipOutline, ConstructOutline, ColorPaletteOutline,
  ListOutline, AppsOutline, FlashOutline, HeartOutline,
  StarOutline, LeafOutline, CogOutline, CubeOutline,
  EyeOutline, LockClosedOutline, TimeOutline, TrendingUpOutline,
  GridOutline, BookOutline, CodeSlashOutline, TerminalOutline,
  MenuOutline, InformationCircleOutline, RefreshOutline
} from '@vicons/ionicons5'
// 与后端共用同一份默认配置（Vite 构建时静态打包）
import defaultsFile from '../../../menu-defaults.json'

export const iconComponents = {
  StatsChart, AnalyticsOutline, ThermometerOutline, ReaderOutline,
  PeopleOutline, PulseOutline, GitNetwork, WifiOutline,
  LayersOutline, GitCompareOutline, BarChartOutline, SpeedometerOutline,
  ShieldOutline, PlanetOutline, FunnelOutline, KeyOutline,
  SettingsOutline, ServerOutline, CalendarOutline, ArchiveOutline,
  PricetagsOutline, CloudOutline, RocketOutline, ExtensionPuzzleOutline,
  HomeOutline, HardwareChipOutline, ConstructOutline, ColorPaletteOutline,
  ListOutline, AppsOutline, FlashOutline, HeartOutline,
  StarOutline, LeafOutline, CogOutline, CubeOutline,
  EyeOutline, LockClosedOutline, TimeOutline, TrendingUpOutline,
  GridOutline, BookOutline, CodeSlashOutline, TerminalOutline,
  MenuOutline, InformationCircleOutline, RefreshOutline
}

export function renderIcon(name) {
  const comp = iconComponents[name] || ExtensionPuzzleOutline
  return () => h(NIcon, null, { default: () => h(comp) })
}

export const fallbackIcons = defaultsFile.icons
export const fallbackMenu = {
  brand: defaultsFile.brand,
  groups: defaultsFile.groups
}

export function cloneMenu(cfg) {
  return JSON.parse(JSON.stringify(cfg || fallbackMenu))
}
