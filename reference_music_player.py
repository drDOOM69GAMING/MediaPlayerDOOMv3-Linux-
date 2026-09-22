#!/usr/bin/env python3
import sys
import os
import io
import subprocess

REQUIRED_PYTHON_VERSION = (3, 9)

if sys.version_info < REQUIRED_PYTHON_VERSION:
    print(f"Python {REQUIRED_PYTHON_VERSION[0]}.{REQUIRED_PYTHON_VERSION[1]}+ required.")
    sys.exit(1)

from pathlib import Path
from dataclasses import dataclass, field
from typing import Optional
import logging
import re
import json
import random
import datetime
import threading

try:
    import tkinter as tk
    from tkinter import ttk, filedialog
except ImportError:
    print("tkinter not available. Install python3-tk.")
    sys.exit(1)

try:
    import pygame
except ImportError:
    print("pygame not installed. Run: pip install pygame")
    sys.exit(1)

try:
    from pydub import AudioSegment
    from pydub.playback import play
    PYDUB_AVAILABLE = True
except ImportError:
    print("pydub not installed. Run: pip install pydub")
    PYDUB_AVAILABLE = False

try:
    import numpy as np
    from scipy import signal
    SCIPY_AVAILABLE = True
except ImportError:
    print("numpy/scipy not installed. Run: pip install numpy scipy for EQ")
    SCIPY_AVAILABLE = False

try:
    import keyboard
    KEYBOARD_AVAILABLE = True
except ImportError:
    KEYBOARD_AVAILABLE = False

try:
    import requests
    REQUESTS_AVAILABLE = True
except ImportError:
    REQUESTS_AVAILABLE = False

try:
    from mutagen import File as MutagenFile
    from mutagen.mp3 import MP3
    from mutagen.easyid3 import EasyID3
except ImportError:
    print("mutagen not installed. Run: pip install mutagen")
    sys.exit(1)

try:
    from PIL import Image, ImageTk, ImageDraw
    PIL_AVAILABLE = True
except ImportError:
    PIL_AVAILABLE = False

try:
    import pystray
    PYSTRAY_AVAILABLE = True
except ImportError:
    PYSTRAY_AVAILABLE = False

logging.basicConfig(
    filename=Path.home() / "music_player.log",
    level=logging.DEBUG,
    format="%(asctime)s - %(levelname)s - %(message)s"
)
logger = logging.getLogger(__name__)

DEBUG_MODE = True

AUDIO_FORMATS = {".mp3", ".wav", ".flac", ".m4a", ".m4b", ".m4p", ".mpc", ".ogg", ".oga", ".mogg",
                 ".raw", ".wma", ".wv", ".webm", ".cda", ".3gp", ".aa", ".aac", ".aax",
                 ".alac", ".aiff", ".dsd", ".mqa"}

THEMES = {
    "Matrix": {
        "bg": "#1e1e1e", "fg": "#00ff00", "accent": "#00aa00", "highlight": "#003300",
        "btn_bg": "#2a2a2a", "btn_fg": "#00ff00", "playing_fg": "#00ff00", "art_bg": "#002200"
    },
    "Cyberpunk": {
        "bg": "#0d0221", "fg": "#ff2a6d", "accent": "#05d9e8", "highlight": "#1a1a3a",
        "btn_bg": "#1a0a2e", "btn_fg": "#ff2a6d", "playing_fg": "#05d9e8", "art_bg": "#1a0030"
    },
    "Amber": {
        "bg": "#1a1400", "fg": "#ffb000", "accent": "#cc8800", "highlight": "#332200",
        "btn_bg": "#2a2000", "btn_fg": "#ffb000", "playing_fg": "#ffdd00", "art_bg": "#221100"
    },
    "Ocean": {
        "bg": "#0a1628", "fg": "#00d4ff", "accent": "#0088aa", "highlight": "#0f2844",
        "btn_bg": "#122a4a", "btn_fg": "#00d4ff", "playing_fg": "#00ffff", "art_bg": "#061020"
    }
}


@dataclass
class PlayerState:
    volume: int = 50
    current_song: Optional[str] = None
    playlist: list = field(default_factory=list)
    prev_songs: list = field(default_factory=list)
    repeat_enabled: bool = False
    playlist_only_mode: bool = False
    playlist_only_sequential: bool = False
    dir_sequential: bool = False
    is_paused: bool = False
    song_count: int = 0
    skip_count: int = 0
    start_time: Optional[datetime.datetime] = None
    current_dir: Optional[str] = None
    current_theme: str = "Matrix"
    song_length_ms: int = 0
    current_pos_offset: float = 0.0
    play_history: dict = field(default_factory=dict)
    song_weights: dict = field(default_factory=dict)
    sleep_timer_minutes: int = 0
    eq_preset: str = "Flat"


EQ_PRESETS = {
    "Flat": {"60Hz": 0, "170Hz": 0, "310Hz": 0, "600Hz": 0, "1kHz": 0, "3kHz": 0, "6kHz": 0, "12kHz": 0, "14kHz": 0, "16kHz": 0},
    "Metal": {"60Hz": 5, "170Hz": 4, "310Hz": 2, "600Hz": -1, "1kHz": 3, "3kHz": 5, "6kHz": 6, "12kHz": 5, "14kHz": 4, "16kHz": 3},
    "Rock": {"60Hz": 4, "170Hz": 3, "310Hz": 2, "600Hz": 1, "1kHz": 2, "3kHz": 4, "6kHz": 5, "12kHz": 4, "14kHz": 3, "16kHz": 2},
    "Pop": {"60Hz": -2, "170Hz": 0, "310Hz": 3, "600Hz": 5, "1kHz": 4, "3kHz": 2, "6kHz": 0, "12kHz": -1, "14kHz": 0, "16kHz": 1},
    "Heavy Metal": {"60Hz": 6, "170Hz": 5, "310Hz": 3, "600Hz": -2, "1kHz": 2, "3kHz": 6, "6kHz": 7, "12kHz": 6, "14kHz": 5, "16kHz": 4},
    "Death Metal": {"60Hz": 7, "170Hz": 6, "310Hz": 4, "600Hz": -3, "1kHz": 1, "3kHz": 7, "6kHz": 8, "12kHz": 6, "14kHz": 5, "16kHz": 4},
    "Punk Rock": {"60Hz": 4, "170Hz": 2, "310Hz": 1, "600Hz": 0, "1kHz": 2, "3kHz": 4, "6kHz": 5, "12kHz": 4, "14kHz": 3, "16kHz": 2},
    "Bass Boost": {"60Hz": 8, "170Hz": 6, "310Hz": 4, "600Hz": 2, "1kHz": 0, "3kHz": 0, "6kHz": 0, "12kHz": 0, "14kHz": 0, "16kHz": 0},
    "Treble Boost": {"60Hz": 0, "170Hz": 0, "310Hz": 0, "600Hz": 0, "1kHz": 2, "3kHz": 4, "6kHz": 6, "12kHz": 8, "14kHz": 9, "16kHz": 10},
    "Vocal": {"60Hz": -2, "170Hz": -1, "310Hz": 2, "600Hz": 4, "1kHz": 5, "3kHz": 4, "6kHz": 2, "12kHz": 1, "14kHz": 0, "16kHz": 0},
}


class MusicPlayerApp:
    APP_NAME = "Random Shuffle Player"
    SETTINGS_FILE = Path.home() / ".music_player_settings.json"
    HISTORY_FILE = Path.home() / ".music_player_history.json"

    SONG_END_EVENT = 26

    def __init__(self):
        pygame.init()
        pygame.mixer.init()
        pygame.mixer.music.set_endevent(self.SONG_END_EVENT)
        self.state = PlayerState()
        self.ui_elements = {}
        self.tray = None
        self.skip_cooldown = False
        self.progress_dragging = False
        self.sleep_timer_id = None
        self.current_album_art = None
        self.album_image_label = None
        self.visualizer_bars = []
        self._song_ended_pending = False
        self._is_loading = False
        self._lock = threading.Lock()

        self.root = tk.Tk()
        self.root.title(self.APP_NAME)
        self.root.configure(bg=self._theme()["bg"])
        self.root.geometry("950x820")
        self.root.attributes('-fullscreen', True)
        self.root.protocol("WM_DELETE_WINDOW", self.on_close)

        self._setup_ui()
        self._setup_hotkeys()
        self._start_auto_load()
        self._start_update_loop()
        self._setup_tray()
        self.load_settings()

    def load_settings(self):
        if self.SETTINGS_FILE.exists():
            try:
                data = json.loads(self.SETTINGS_FILE.read_text())
                self.state.volume = data.get("volume", 50)
                self.state.current_song = data.get("last_played")
                self.state.playlist = data.get("playlist", [])
                self.state.current_theme = data.get("theme", "Matrix")
                self.set_volume(self.state.volume)
            except Exception as e:
                logger.error(f"Settings load error: {e}")

    def save_settings(self):
        try:
            data = {
                "volume": self.state.volume,
                "last_played": self.state.current_song,
                "playlist": self.state.playlist,
                "theme": self.state.current_theme
            }
            self.SETTINGS_FILE.write_text(json.dumps(data, indent=2))
        except Exception as e:
            logger.error(f"Settings save error: {e}")

    def load_history(self):
        if self.HISTORY_FILE.exists():
            try:
                self.state.play_history = json.loads(self.HISTORY_FILE.read_text())
            except:
                pass

    def save_history(self):
        try:
            self.HISTORY_FILE.write_text(json.dumps(self.state.play_history, indent=2))
        except:
            pass

    def get_audio_files(self, directory: str) -> list:
        dir_path = Path(directory)
        if not dir_path.exists():
            return []
        return sorted([
            str(f) for f in dir_path.rglob("*")
            if f.is_file() and f.suffix.lower() in AUDIO_FORMATS
        ])

    def get_song_duration(self, song_path: str) -> int:
        try:
            audio = MP3(song_path)
            return int(audio.info.length * 1000)
        except:
            return 0

    def get_album_art(self, song_path: str) -> Optional[Image.Image]:
        if not PIL_AVAILABLE:
            return None
        song_dir = Path(song_path).parent
        art_names = ["cover.jpg", "cover.png", "folder.jpg", "album.jpg", "front.jpg", "default.jpg"]
        for name in art_names:
            art_path = song_dir / name
            if art_path.exists():
                try:
                    return Image.open(art_path).convert('RGB')
                except:
                    pass
        for ext in ["*.jpg", "*.jpeg", "*.png"]:
            for img_file in song_dir.glob(ext):
                try:
                    return Image.open(img_file).convert('RGB')
                except:
                    pass
        try:
            audio = MutagenFile(song_path)
            if audio and hasattr(audio, 'pictures') and audio.pictures:
                for pic in audio.pictures:
                    img_data = io.BytesIO(pic.data)
                    return Image.open(img_data).convert('RGB')
        except:
            pass
        return None

    def get_metadata(self, song_path: str) -> dict:
        try:
            audio = MP3(song_path, ID3=EasyID3)
            return {
                'title': audio.get('title', [Path(song_path).stem])[0],
                'artist': audio.get('artist', ['Unknown'])[0],
                'album': audio.get('album', ['Unknown'])[0]
            }
        except:
            return {'title': Path(song_path).stem, 'artist': 'Unknown', 'album': 'Unknown'}

    def fetch_web_art(self, artist: str, album: str) -> Optional[Image.Image]:
        if not REQUESTS_AVAILABLE or not PIL_AVAILABLE:
            return None
        try:
            from urllib.parse import quote
            query = quote(f"{artist} {album}")
            url = f"https://itunes.apple.com/search?term={query}&entity=album&limit=1"
            response = requests.get(url, timeout=5)
            data = response.json()
            if data["resultCount"] > 0:
                art_url = data["results"][0]["artworkUrl100"].replace("100x100bb", "500x500bb")
                img_res = requests.get(art_url)
                return Image.open(io.BytesIO(img_res.content)).convert('RGB')
        except:
            pass
        return None

    def find_music_folder(self) -> Optional[str]:
        candidates = []
        def check_folder(folder_path, priority=0):
            try:
                if folder_path.exists() and folder_path.is_dir():
                    files = self.get_audio_files(str(folder_path))
                    if files:
                        candidates.append((folder_path, len(files) + priority * 10000))
            except PermissionError:
                pass
        check_folder(Path.home() / "Music", priority=1)
        for mount_base in ["/media", "/mnt", "/run/media"]:
            base_path = Path(mount_base)
            if not base_path.exists():
                continue
            try:
                for user in base_path.iterdir():
                    if user.is_dir():
                        check_folder(user / "Music", priority=2)
                        try:
                            for item in user.iterdir():
                                if item.is_dir():
                                    if item.name.lower() == "music":
                                        check_folder(item, priority=2)
                                    for sub in item.iterdir():
                                        if sub.is_dir() and sub.name.lower() == "music":
                                            check_folder(sub, priority=2)
                        except PermissionError:
                            pass
            except PermissionError:
                continue
        if candidates:
            best = max(candidates, key=lambda x: x[1])
            return str(best[0])
        return None

    def _start_auto_load(self):
        def load():
            music_folder = self.find_music_folder()
            if music_folder:
                self.root.after(0, lambda: self.load_directory(str(music_folder)))
                self.root.after(0, lambda: self.set_status(f"Auto-loaded: {music_folder}"))
        threading.Thread(target=load, daemon=True).start()

    def load_directory(self, directory: str):
        self.set_status("Scanning library...")
        threading.Thread(target=self._background_load, args=(directory,), daemon=True).start()

    def _background_load(self, directory: str):
        files = self.get_audio_files(directory)
        self.root.after(0, lambda: self._finalize_load(directory, files))

    def _finalize_load(self, directory: str, files: list):
        self.state.current_dir = directory
        if files:
            self.state.playlist = files
            self.state.song_count = 0
            self.state.skip_count = 0
            self.state.start_time = datetime.datetime.now()
            self.update_playlist_ui()
            self.set_status(f"Loaded {len(files)} songs")
            self.skip_song()
        else:
            self.set_status("No songs found")

    def change_directory(self):
        dir_path = filedialog.askdirectory(initialdir=str(Path.home()))
        if dir_path:
            self.load_directory(dir_path)

    def apply_eq(self, audio_segment, preset_name):
        if not SCIPY_AVAILABLE or preset_name == "Flat":
            return audio_segment

        preset = EQ_PRESETS.get(preset_name, EQ_PRESETS["Flat"])

        samples = np.array(audio_segment.get_array_of_samples(), dtype=np.float32)
        max_sample = np.iinfo(np.int16).max
        samples = samples / max_sample

        if audio_segment.channels == 2:
            samples = samples.reshape((-1, 2))
            left = samples[:, 0].copy()
            right = samples[:, 1].copy()
        else:
            left = samples.copy()
            right = samples.copy()

        sample_rate = audio_segment.frame_rate
        nyquist = sample_rate / 2

        freq_map = {
            "60Hz": 60, "170Hz": 170, "310Hz": 310, "600Hz": 600, "1kHz": 1000,
            "3kHz": 3000, "6kHz": 6000, "12kHz": 12000, "14kHz": 14000, "16kHz": 16000
        }

        for name, boost in preset.items():
            if abs(boost) < 1:
                continue
            freq = freq_map.get(name, 1000)
            freq = min(freq, nyquist - 100)

            gain = boost * 0.25

            if boost > 0:
                low_freq = max(20, freq - 200)
                b, a = signal.butter(2, [low_freq / nyquist, freq / nyquist], btype='band')
                filtered = signal.lfilter(b, a, left)
                left = left + gain * filtered
                right = right + gain * filtered
            else:
                b, a = signal.butter(2, freq / nyquist, btype='low')
                filtered = signal.lfilter(b, a, left)
                left = left + gain * filtered
                right = right + gain * filtered

        left = np.clip(left, -1.0, 1.0)
        right = np.clip(right, -1.0, 1.0)

        if audio_segment.channels == 2:
            stereo = np.column_stack((left, right)).flatten()
        else:
            stereo = left

        stereo = (stereo * max_sample).astype(np.int16)
        return audio_segment._spawn(stereo.tobytes())

    def play_song(self, song_path: str):
        logger.info(f"play_song called: {song_path}, is_loading={self._is_loading}")
        if self._is_loading:
            logger.warning("play_song: already loading, returning")
            return
        
        if not song_path or not os.path.exists(song_path):
            logger.error(f"play_song: file not found: {song_path}")
            self.set_error("File not found")
            return

        pygame.event.clear(self.SONG_END_EVENT)

        with self._lock:
            self._is_loading = True
            try:
                logger.info("play_song: stopping mixer")
                pygame.mixer.music.stop()
                try:
                    pygame.mixer.music.unload()
                except Exception as e:
                    logger.warning(f"play_song: unload error: {e}")
                
                self.state.current_song = song_path
                self.state.prev_songs.append(song_path)
                self.state.song_count += 1
                self.state.song_length_ms = self.get_song_duration(song_path)
                self.state.current_pos_offset = 0.0
                self.state.play_history[song_path] = self.state.play_history.get(song_path, 0) + 1
                self.save_history()

                self.update_playing_label(song_path)
                self.set_status(f"Loading: {Path(song_path).stem}...")

                if PYDUB_AVAILABLE and SCIPY_AVAILABLE and self.state.eq_preset != "Flat":
                    logger.info("play_song: starting EQ thread")
                    threading.Thread(target=self._play_with_eq, args=(song_path,), daemon=True).start()
                else:
                    logger.info(f"play_song: loading directly: {song_path}")
                    pygame.mixer.music.load(song_path)
                    pygame.mixer.music.play()
                    self._finish_song_setup(song_path)

            except Exception as e:
                logger.error(f"play_song EXCEPTION: {e}", exc_info=True)
                self.set_error(f"Error: {e}")
            finally:
                self._is_loading = False
                logger.info("play_song: finished, is_loading=False")
                self._is_loading = False

    def _play_with_eq(self, song_path):
        try:
            audio = AudioSegment.from_file(song_path)
            audio = self.apply_eq(audio, self.state.eq_preset)
            audio.export("/tmp/eq_audio.mp3", format="mp3", bitrate="192k")
            pygame.mixer.music.load("/tmp/eq_audio.mp3")
            pygame.mixer.music.play()
            self.root.after(0, lambda: self._finish_song_setup(song_path))
        except Exception as e:
            logger.error(f"EQ playback error: {e}")
            pygame.mixer.music.load(song_path)
            pygame.mixer.music.play()
            self.root.after(0, lambda: self._finish_song_setup(song_path))

    def _finish_song_setup(self, song_path):
        self.update_album_art(song_path)
        self.update_metadata(song_path)
        meta = self.get_metadata(song_path)
        title_str = f"♪ {meta['artist']} - {meta['title']}"
        self.root.title(title_str)
        self.save_settings()
        self.set_status(f"Playing: {Path(song_path).stem}")
        self._draw_progress_bar(0)
        self._update_progress_from_mixer()
        self._update_tray_tooltip()
        self._update_visualizer()
        self._is_loading = False

        lb = self.ui_elements["playlist"]
        try:
            idx = self.state.playlist.index(song_path)
            lb.selection_clear(0, tk.END)
            lb.selection_set(idx)
            lb.see(idx)
        except ValueError:
            pass

    def skip_song(self):
        logger.info(f"skip_song called, playlist_len={len(self.state.playlist) if self.state.playlist else 0}")
        current_time = pygame.mixer.music.get_pos() / 1000
        if self.state.current_song and current_time < 10:
            self.state.song_weights[self.state.current_song] = self.state.song_weights.get(self.state.current_song, 1.0) * 0.8

        if not self.state.playlist or self.state.is_paused:
            logger.warning(f"skip_song: no playlist or paused, returning")
            return
        playlist = self.state.playlist
        if self.state.playlist_only_mode and playlist:
            if self.state.playlist_only_sequential:
                song = playlist[self.state.song_count % len(playlist)]
            else:
                weights = [self.state.song_weights.get(s, 1.0) for s in playlist]
                song = random.choices(playlist, weights=weights, k=1)[0]
        elif self.state.dir_sequential and self.state.current_dir:
            files = self.get_audio_files(self.state.current_dir)
            if files:
                song = files[self.state.song_count % len(files)]
            else:
                song = random.choice(playlist)
        elif playlist:
            weights = [self.state.song_weights.get(s, 1.0) for s in playlist]
            song = random.choices(playlist, weights=weights, k=1)[0]
        else:
            logger.warning(f"skip_song: no song found")
            return
        self.state.skip_count += 1
        logger.info(f"skip_song: calling play_song with: {song}")
        self.play_song(song)

    def prev_song(self):
        if len(self.state.prev_songs) > 1:
            self.state.prev_songs.pop()
            self.play_song(self.state.prev_songs[-1])

    def pause_music(self):
        self.state.is_paused = True
        pygame.mixer.music.pause()
        self.ui_elements["pause_btn"].config(text="Paused", fg=self._theme()["accent"])
        self.set_status("Paused")
        self._update_tray_tooltip()

    def unpause_music(self):
        if self.state.current_song:
            self.state.is_paused = False
            pygame.mixer.music.unpause()
            self.ui_elements["pause_btn"].config(text="Pause", fg=self._theme()["btn_fg"])
            self.set_status("Playing")
            self._update_tray_tooltip()

    def skip_forward(self):
        if self.state.current_song and not self.state.is_paused:
            current_pos = pygame.mixer.music.get_pos()
            if current_pos >= 0:
                new_pos = (current_pos / 1000) + 10
                pygame.mixer.music.set_pos(new_pos)

    def skip_backward(self):
        if self.state.current_song and not self.state.is_paused:
            current_pos = pygame.mixer.music.get_pos()
            if current_pos >= 0:
                new_pos = (current_pos / 1000) - 10
                pygame.mixer.music.set_pos(max(0, new_pos))

    def toggle_pause(self):
        if self.state.is_paused:
            self.unpause_music()
        else:
            self.pause_music()

    def toggle_repeat(self):
        self.state.repeat_enabled = not self.state.repeat_enabled
        self.ui_elements["repeat_btn"].config(
            text=f"Repeat: {'On' if self.state.repeat_enabled else 'Off'}",
            fg=self._theme()["accent"] if self.state.repeat_enabled else self._theme()["btn_fg"]
        )
        self.set_status(f"Repeat {'On' if self.state.repeat_enabled else 'Off'}")

    def cycle_eq_preset(self):
        presets = list(EQ_PRESETS.keys())
        idx = presets.index(self.state.eq_preset) if self.state.eq_preset in presets else 0
        self.state.eq_preset = presets[(idx + 1) % len(presets)]
        self.ui_elements["eq_btn"].config(text=f"EQ: {self.state.eq_preset}")
        self.set_status(f"EQ: {self.state.eq_preset}")
        try:
            import pygame.mixer
            pygame.mixer.init()
            volumes = EQ_PRESETS[self.state.eq_preset]
            base_vol = self.state.volume / 100
            for freq, boost in volumes.items():
                pass
        except:
            pass

    def toggle_playlist_only(self):
        self.state.playlist_only_mode = not self.state.playlist_only_mode
        self.ui_elements["playlist_only_btn"].config(
            text=f"Playlist Only: {'On' if self.state.playlist_only_mode else 'Off'}",
            fg=self._theme()["accent"] if self.state.playlist_only_mode else self._theme()["btn_fg"]
        )

    def toggle_playlist_sequential(self):
        self.state.playlist_only_sequential = not self.state.playlist_only_sequential
        self.ui_elements["playlist_seq_btn"].config(
            text=f"Sequential: {'On' if self.state.playlist_only_sequential else 'Off'}",
            fg=self._theme()["accent"] if self.state.playlist_only_sequential else self._theme()["btn_fg"]
        )

    def toggle_dir_sequential(self):
        self.state.dir_sequential = not self.state.dir_sequential
        self.ui_elements["dir_seq_btn"].config(
            text=f"Dir Seq: {'On' if self.state.dir_sequential else 'Off'}",
            fg=self._theme()["accent"] if self.state.dir_sequential else self._theme()["btn_fg"]
        )

    def cycle_theme(self):
        themes = list(THEMES.keys())
        idx = themes.index(self.state.current_theme) if self.state.current_theme in themes else 0
        self.state.current_theme = themes[(idx + 1) % len(themes)]
        self._apply_theme()
        self.save_settings()
        self.set_status(f"Theme: {self.state.current_theme}")

    def _theme(self):
        return THEMES.get(self.state.current_theme, THEMES["Matrix"])

    def _apply_theme(self):
        t = self._theme()
        self.root.configure(bg=t["bg"])

    def smart_shuffle(self):
        if len(self.state.playlist) < 3:
            self.set_error("Need more songs in playlist")
            return
        weights = [self.state.song_weights.get(s, 1.0) for s in self.state.playlist]
        if sum(weights) <= 0:
            self.set_error("No weighted songs yet, play more!")
            return
        self.set_status(f"Smart shuffle: {len(self.state.playlist)} songs")

    def set_sleep_timer(self, minutes: int):
        if self.sleep_timer_id:
            self.root.after_cancel(self.sleep_timer_id)
            self.sleep_timer_id = None
        if minutes > 0:
            self.state.sleep_timer_minutes = minutes
            ms = minutes * 60 * 1000
            self.sleep_timer_id = self.root.after(ms, self._sleep_timer_action)
            self.set_status(f"Sleep timer: {minutes} min")
        else:
            self.state.sleep_timer_minutes = 0
            self.set_status("Sleep timer off")

    def _sleep_timer_action(self):
        pygame.mixer.music.fadeout(3000)
        self.root.after(4000, self.on_close)

    def save_song(self):
        if self.state.current_song and self.state.current_song not in self.state.playlist:
            self.state.playlist.append(self.state.current_song)
            self.update_playlist_ui()
            self.save_settings()
            self.set_status("Song saved to playlist")

    def clear_playlist(self):
        self.state.playlist = []
        self.update_playlist_ui()
        self.save_settings()
        self.set_status("Playlist cleared")

    def shuffle_playlist(self):
        random.shuffle(self.state.playlist)
        self.update_playlist_ui()
        self.save_settings()
        self.set_status("Playlist shuffled")

    def save_playlist_file(self):
        try:
            file_path = filedialog.asksaveasfilename(
                initialfile="playlist.json",
                defaultextension=".json",
                filetypes=[("JSON", "*.json")]
            )
            if file_path:
                Path(file_path).write_text(json.dumps(self.state.playlist, indent=2))
                self.set_status("Playlist saved")
        except Exception as e:
            logger.error(f"Save playlist error: {e}")
            self.set_error(f"Error: {e}")

    def load_playlist_file(self):
        try:
            file_path = filedialog.askopenfilename(
                filetypes=[("JSON", "*.json"), ("All", "*.*")]
            )
            if file_path:
                self.state.playlist = json.loads(Path(file_path).read_text())
                self.update_playlist_ui()
                self.set_status("Playlist loaded")
        except Exception as e:
            logger.error(f"Load playlist error: {e}")
            self.set_error(f"Error: {e}")

    def set_volume(self, level: int):
        self.state.volume = max(0, min(100, level))
        pygame.mixer.music.set_volume(self.state.volume / 100)
        if "volume_label" in self.ui_elements:
            self.ui_elements["volume_label"].config(text=f"Volume: {self.state.volume}%")
        self.save_settings()

    def increase_volume(self):
        self.set_volume(self.state.volume + 5)

    def decrease_volume(self):
        self.set_volume(self.state.volume - 5)

    def lookup_song(self):
        if not self.state.current_song:
            return
        try:
            folder = os.path.dirname(self.state.current_song)
            if folder:
                subprocess.run(["xdg-open", folder])
        except Exception as e:
            logger.error(f"Lookup error: {e}")
            self.set_error(f"Error opening folder: {e}")

    def play_video(self):
        if not self.state.current_song:
            return
        folder = os.path.dirname(self.state.current_song)
        song_name = os.path.splitext(os.path.basename(self.state.current_song))[0]
        for ext in ['.mp4', '.mkv', '.avi', '.webm']:
            video_path = os.path.join(folder, song_name + ext)
            if Path(video_path).exists():
                subprocess.Popen(["xdg-open", video_path])
                return
        for ext in ['.mp4', '.mkv', '.avi', '.webm']:
            video_path = filedialog.askopenfilename(
                initialdir=folder,
                filetypes=[("Video", f"*{ext}")]
            )
            if video_path:
                subprocess.Popen(["xdg-open", video_path])
                return
        self.set_error("No matching video found")

    def show_help(self):
        t = self._theme()
        help_win = tk.Toplevel(self.root)
        help_win.title("SYSTEM MANUAL")
        help_win.configure(bg="#000000")
        help_win.geometry("450x400")
        
        help_text = """[ OPERATIONAL COMMANDS ]
------------------------
SPACE   : Play / Pause
LEFT    : Previous Track
RIGHT   : Skip Track
S       : Smart Shuffle
T       : Cycle Theme
F       : Toggle Fullscreen
UP/DOWN : Volume Control
+ / -  : Volume Buttons

[ PLAYLIST ]
----------
Click   : Play Selected
Drag    : Add Files/Folders

[ FEATURES ]
-----------
Sleep   : Auto-close timer
Look Up : Open song folder
EQ      : Equalizer presets"""
        
        label = tk.Label(help_win, text=help_text, fg="#00FF00", bg="#000000", 
                         font=("Courier", 11), justify=tk.LEFT, padx=30, pady=30)
        label.pack()
        tk.Button(help_win, text="CLOSE", command=help_win.destroy,
                 bg="#003300", fg="#00FF00", font=("Courier", 10, "bold")).pack(pady=10)

    def fetch_lyrics(self):
        if not self.state.current_song:
            return
        meta = self.get_metadata(self.state.current_song)
        artist = meta["artist"]
        title = meta["title"]
        self.set_status(f"Fetching lyrics...")
        threading.Thread(target=self._fetch_lyrics_async, args=(artist, title), daemon=True).start()

    def _fetch_lyrics_async(self, artist: str, title: str):
        try:
            import requests
            query = f"{artist} {title} lyrics".replace(" ", "%20")
            url = f"https://api.lyrics.ovh/v1/{artist}/{title}"
            resp = requests.get(url, timeout=10)
            if resp.status_code == 200:
                data = resp.json()
                lyrics = data.get("lyrics", "")
                self.root.after(0, lambda: self._show_lyrics_window(artist, title, lyrics))
            else:
                self.root.after(0, lambda: self.set_error("Lyrics not found"))
        except Exception as e:
            self.root.after(0, lambda: self.set_error(f"Lyrics error: {e}"))

    def _show_lyrics_window(self, artist: str, title: str, lyrics: str):
        win = tk.Toplevel(self.root)
        win.title(f"{artist} - {title}")
        win.geometry("600x500")
        win.configure(bg="#000000")
        
        text = tk.Text(win, bg="#000000", fg="#39FF14", font=("Courier", 11), wrap=tk.WORD)
        text.pack(padx=20, pady=20, fill=tk.BOTH, expand=True)
        text.insert(tk.END, lyrics)
        text.config(state=tk.DISABLED)
        
        text.bind("<Button-4>", lambda e: text.yview_scroll(-1, "units"))
        text.bind("<Button-5>", lambda e: text.yview_scroll(1, "units"))
        
        tk.Button(win, text="CLOSE", command=win.destroy,
                bg="#003300", fg="#00FF00", font=("Courier", 10, "bold")).pack(pady=10)

    def on_drop(self, event):
        files = self.root.tk.splitlist(event.data)
        self.set_status(f"Adding files...")
        threading.Thread(target=self._background_drop, args=(files,), daemon=True).start()

    def _background_drop(self, files: list):
        added = 0
        for f in files:
            path = Path(f)
            if path.is_dir():
                songs = self.get_audio_files(str(path))
                self.state.playlist.extend(songs)
                added += len(songs)
            elif path.suffix.lower() in AUDIO_FORMATS:
                if str(path) not in self.state.playlist:
                    self.state.playlist.append(str(path))
                    added += 1
        self.state.playlist = sorted(set(self.state.playlist))
        self.root.after(0, lambda: self._finalize_drop(added))

    def _finalize_drop(self, count: int):
        self.update_playlist_ui()
        self.save_settings()
        self.set_status(f"Added {count} files")

    def play_selected(self, event):
        idx = self.ui_elements["playlist"].curselection()
        if idx:
            song = self.state.playlist[idx[0]] if idx[0] < len(self.state.playlist) else None
            if song:
                self.play_song(song)

    def update_album_art(self, song_path: str):
        if not PIL_AVAILABLE or not self.album_image_label:
            return
        try:
            if self.album_image_label:
                self.album_image_label.config(image="", text="Loading art...")
            threading.Thread(target=self._load_album_art, args=(song_path,), daemon=True).start()
        except Exception as e:
            print(f"Album art error: {e}")

    def _load_album_art(self, song_path):
        try:
            print(f"Loading art for: {song_path}")
            img = self.get_album_art(song_path)
            print(f"Got local art: {img}")
            if not img and REQUESTS_AVAILABLE:
                meta = self.get_metadata(song_path)
                if meta['artist'] != 'Unknown' and meta['album'] != 'Unknown':
                    img = self.fetch_web_art(meta['artist'], meta['album'])
                    print(f"Got web art: {img}")

            def update_ui():
                if img and self.album_image_label:
                    img_resized = img.resize((200, 200), Image.Resampling.LANCZOS)
                    self.current_album_art = ImageTk.PhotoImage(img_resized)
                    self.album_image_label.config(image=self.current_album_art, text="")
                elif self.album_image_label:
                    self.album_image_label.config(image="", text="No Art")

            self.root.after(0, update_ui)
        except Exception as e:
            print(f"Album art loading error: {e}")

    def update_metadata(self, song_path: str):
        meta = self.get_metadata(song_path)
        self.ui_elements["meta_title"].config(text=meta['title'])
        self.ui_elements["meta_artist"].config(text=meta['artist'])
        self.ui_elements["meta_album"].config(text=meta['album'])

    def update_playing_label(self, song_path: str):
        p = Path(song_path)
        name = self.beautify_name(p.stem)
        display = f"{p.parent.name} > {name}"
        self.ui_elements["playing"].config(text=display, fg=self._theme()["playing_fg"])

    def beautify_name(self, filename: str) -> str:
        name = re.sub(r'^(CD\s?\d+[-]?\s*|\d+[-.]?\s*)+', '', filename)
        name = re.sub(r'\s*\([^)]*\)\s*', '', name)
        name = re.sub(r'[\[\(]?\d{3}\s?(kbps|kb)?[\)]?$', '', name)
        name = re.sub(r'\s*-\s*\d{4}\s*$', '', name)
        return name.strip() or filename

    def update_playlist_ui(self):
        lb = self.ui_elements["playlist"]
        lb.delete(0, tk.END)
        for song in self.state.playlist:
            path = Path(song)
            folder = path.parent.name
            name = self.beautify_name(path.stem)
            display = f"{folder} - {name}"
            lb.insert(tk.END, display)

    def on_search(self, event):
        query = self.ui_elements["search_entry"].get().lower()
        lb = self.ui_elements["playlist"]
        lb.delete(0, tk.END)
        for song in self.state.playlist:
            if query in song.lower():
                path = Path(song)
                folder = path.parent.name
                name = self.beautify_name(path.stem)
                display = f"{folder} - {name}"
                lb.insert(tk.END, display)

    def set_status(self, text: str):
        lb = self.ui_elements["playlist"]
        try:
            idx = self.state.playlist.index(self.state.current_song) if self.state.current_song else -1
            if idx >= 0:
                lb.selection_clear(0, tk.END)
                lb.selection_set(idx)
                lb.see(idx)
        except ValueError:
            pass
        self.ui_elements["status"].config(text=text)
        self.root.after(5000, lambda: self.ui_elements["status"].config(text=self.APP_NAME))

    def set_error(self, text: str):
        self.ui_elements["error"].config(text=text)
        self.root.after(5000, lambda: self.ui_elements["error"].config(text=""))

    def _start_update_loop(self):
        self._update_labels()
        self.root.after(100, self._start_update_loop)

    def _update_labels(self):
        if self.state.start_time:
            running = datetime.datetime.now() - self.state.start_time
            info = f"Songs: {self.state.song_count} | Skips: {self.state.skip_count} | {running}"
            self.ui_elements["info"].config(text=info)

        if not self._is_loading:
            self._update_progress_from_mixer()
            self._update_visualizer()

        if self._song_ended_pending:
            logger.debug("_update_labels: song_ended_pending=True, returning")
            return

        if self._is_loading or self.state.is_paused or not self.state.current_song:
            return

        busy = pygame.mixer.music.get_busy()
        logger.debug(f"_update_labels: busy={busy}, current_song={self.state.current_song is not None}")
        
        if not busy and self.state.current_song:
            logger.info("_update_labels: mixer not busy, calling _handle_song_end")
            self._song_ended_pending = True
            self._handle_song_end()

    def _update_progress_from_mixer(self):
        if self.state.is_paused or self.state.song_length_ms <= 0:
            return

        raw_ms = pygame.mixer.music.get_pos()
        if raw_ms < 0:
            return

        current_sec = int(raw_ms / 1000)
        total_sec = int(self.state.song_length_ms / 1000)
        
        self.ui_elements["progress_label"].config(text=f"{current_sec//60}:{current_sec%60:02d}")
        self.ui_elements["progress_end"].config(text=f"{total_sec//60}:{total_sec%60:02d}")
        
        percent = (raw_ms / self.state.song_length_ms) * 100
        self._draw_progress_bar(min(100, percent))

    def _handle_song_end(self):
        logger.info(f"_handle_song_end: pending={self._song_ended_pending}, repeat={self.state.repeat_enabled}")
        if not self._song_ended_pending:
            return

        now = datetime.datetime.now()
        last_skip = getattr(self, '_last_skip_time', datetime.datetime.min)
        if (now - last_skip).total_seconds() < 1.5:
            logger.info("_handle_song_end: within cooldown, returning")
            return

        self._last_skip_time = now
        self._song_ended_pending = False

        if self.state.repeat_enabled and self.state.current_song:
            logger.info("_handle_song_end: repeating current song")
            self.play_song(self.state.current_song)
        else:
            logger.info("_handle_song_end: skipping to next")
            self.skip_song()

    pass

    def _draw_progress_bar(self, percent):
        canvas = self.ui_elements.get("progress")
        if not canvas:
            return
        t = self._theme()
        canvas.delete("all")
        w = canvas.winfo_width()
        h = canvas.winfo_height()
        if w <= 0 or h <= 0:
            return
        fill = int((percent / 100.0) * w)
        canvas.create_rectangle(0, 0, fill, h, fill=t["accent"], outline="")
        canvas.create_rectangle(fill, 0, w, h, fill=t["highlight"], outline="")
        self.ui_elements["progress_current"] = percent

    def _update_progress_bar(self, value):
        if "progress" in self.ui_elements:
            self.ui_elements["progress"].config(value=value)

    def _scrub_audio(self, event):
        if self.state.song_length_ms <= 0 or not self.state.current_song or self.state.is_paused:
            return
        canvas = self.ui_elements.get("progress")
        if not canvas:
            return
        width = canvas.winfo_width()
        if width > 0:
            pct = max(0, min(1, event.x / width))
            target = pct * self.state.song_length_ms / 1000.0
            try:
                pygame.mixer.music.set_pos(float(target))
            except Exception as e:
                print(f"Seek error: {e}")

    def _on_progress_release(self, event):
        self._scrub_audio(event)

    def _on_progress_scroll(self, event):
        if self.state.song_length_ms > 0 and self.state.current_song and not self.state.is_paused:
            current_pos = pygame.mixer.music.get_pos() / 1000
            if hasattr(event, 'delta') and event.delta > 0 or event.num == 4:
                new_pos = current_pos + 10
            else:
                new_pos = current_pos - 10
            new_pos = max(0, min(self.state.song_length_ms / 1000, new_pos))
            pygame.mixer.music.set_pos(new_pos)

    def _update_visualizer(self):
        if not self.visualizer_bars:
            return
        t = self._theme()
        if self.state.is_paused or not self.state.current_song:
            for bar in self.visualizer_bars:
                bar.configure(height=5, bg=t["highlight"])
        else:
            for bar in self.visualizer_bars:
                h = random.randint(5, 45)
                if self.state.current_theme == "Matrix":
                    color = random.choice([t["fg"], t["accent"], "#ffffff"])
                elif self.state.current_theme == "Cyberpunk":
                    color = random.choice([t["fg"], t["accent"], "#ff00ff"])
                else:
                    color = t["fg"]
                bar.configure(height=h, bg=color)

    def _setup_hotkeys(self):
        self.root.bind("<space>", lambda e: self.toggle_pause())
        self.root.bind("<Right>", lambda e: self.skip_song())
        self.root.bind("<Left>", lambda e: self.prev_song())
        self.root.bind("<Shift-Right>", lambda e: self.skip_forward())
        self.root.bind("<Shift-Left>", lambda e: self.skip_backward())
        self.root.bind("<Up>", lambda e: self.increase_volume())
        self.root.bind("<Down>", lambda e: self.decrease_volume())
        self.root.bind("<plus>", lambda e: self.increase_volume())
        self.root.bind("<KP_Add>", lambda e: self.increase_volume())
        self.root.bind("<minus>", lambda e: self.decrease_volume())
        self.root.bind("<KP_Subtract>", lambda e: self.decrease_volume())
        self.root.bind("s", lambda e: self.smart_shuffle())
        self.root.bind("S", lambda e: self.smart_shuffle())
        self.root.bind("t", lambda e: self.cycle_theme())
        self.root.bind("T", lambda e: self.cycle_theme())
        self.root.bind("<Escape>", lambda e: self.root.attributes('-fullscreen', False))
        self.root.bind("<F10>", lambda e: self.on_close())

        if KEYBOARD_AVAILABLE:
            threading.Thread(target=self._setup_global_hotkeys, daemon=True).start()

    def _setup_global_hotkeys(self):
        try:
            keyboard.add_hotkey('ctrl+alt+m', self._show_window)
            keyboard.add_hotkey('ctrl+alt+q', self._quit_player)
            logger.info("Global hotkeys registered: Ctrl+Alt+M=Show, Ctrl+Alt+Q=Quit")
        except Exception as e:
            logger.error(f"Global hotkeys error: {e}")

    def _setup_tray(self):
        return  # Disabled for debugging
        try:
            def on_show(icon, item):
                try:
                    self.root.after(0, lambda: self.root.deiconify())
                except:
                    pass
            def on_play_pause(icon, item):
                try:
                    self.root.after(0, self.toggle_pause)
                except:
                    pass
            def on_skip(icon, item):
                try:
                    self.root.after(0, self.skip_song)
                except:
                    pass
            def on_prev(icon, item):
                try:
                    self.root.after(0, self.prev_song)
                except:
                    pass
            def on_stop(icon, item):
                try:
                    pygame.mixer.music.stop()
                except:
                    pass
            def on_quit(icon, item):
                try:
                    self._quit_player()
                except:
                    self.root.destroy()
            menu = pystray.Menu(
                pystray.MenuItem("Show", on_show),
                pystray.MenuItem("Play/Pause", on_play_pause),
                pystray.MenuItem("Skip", on_skip),
                pystray.MenuItem("Previous", on_prev),
                pystray.MenuItem("Stop", on_stop),
                pystray.MenuItem("Separator", lambda i, m: None),
                pystray.MenuItem("Quit", on_quit)
            )
            img = Image.new('RGB', (64, 64), color='#1e1e1e')
            draw = ImageDraw.Draw(img)
            draw.ellipse([8, 8, 56, 56], fill='#3a3a3a', outline='#5a5a5a', width=2)
            draw.polygon([(24, 18), (24, 46), (44, 32)], fill='#00ff00')
            
            def run_tray():
                try:
                    self.tray = pystray.Icon("music_player", img, self.APP_NAME, menu)
                    self.tray.run()
                except Exception as e:
                    logger.error(f"Tray error: {e}")
            
            threading.Thread(target=run_tray, daemon=True).start()
        except Exception as e:
            logger.error(f"Tray setup error: {e}")

    def _update_tray_tooltip(self):
        if self.tray:
            song = Path(self.state.current_song).stem if self.state.current_song else "No song"
            paused = "Paused" if self.state.is_paused else "Playing"
            self.tray.title = f"{paused}: {song}\nVol: {self.state.volume}%"

    def on_close(self):
        try:
            pygame.mixer.music.stop()
        except:
            pass
        try:
            pygame.mixer.quit()
        except:
            pass
        self.save_settings()
        if self.tray:
            try:
                self.tray.stop()
            except:
                pass
            self.tray = None
        try:
            self.root.destroy()
        except:
            pass

    def _quit_player(self):
        pygame.mixer.music.stop()
        pygame.mixer.quit()
        self.save_settings()
        if self.tray:
            try:
                self.tray.stop()
            except:
                pass
            self.tray = None
        self.root.destroy()

    def _show_window(self):
        try:
            self.root.after(0, lambda: self.root.deiconify())
            self.root.after(0, lambda: self.root.attributes('-fullscreen', True))
        except Exception as e:
            logger.error(f"Show window error: {e}")

    def run(self):
        self.root.mainloop()

    def _setup_ui(self):
        t = self._theme()
        bg = t["bg"]
        fg = t["fg"]
        btn_bg = t["btn_bg"]
        btn_fg = t["btn_fg"]
        font_ui = ("Courier", 11)
        font_btn = ("Courier", 9, "bold")
        font_title = ("Courier", 14, "bold")
        font_small = ("Courier", 9)
        font_meta = ("Courier", 10)

        self.ui_elements["status"] = tk.Label(self.root, text=self.APP_NAME, bg=bg, fg=fg, font=font_title)
        self.ui_elements["status"].pack(pady=3)

        self.ui_elements["info"] = tk.Label(self.root, text="", bg=bg, fg=fg, font=font_ui)
        self.ui_elements["info"].pack(pady=1)

        self.ui_elements["hotkey_label"] = tk.Label(self.root, text="Space=Play/Pause | <-Prev | ->Skip | +/-Vol | S=Smart | T=Theme", bg=bg, fg="#666", font=font_small)
        self.ui_elements["hotkey_label"].pack(pady=1)

        main_frame = tk.Frame(self.root, bg=bg)
        main_frame.pack(fill=tk.BOTH, expand=True, padx=10)

        left_panel = tk.Frame(main_frame, bg=bg)
        left_panel.pack(side=tk.LEFT, fill=tk.Y, padx=(0, 10))

        art_frame = tk.Frame(left_panel, bg=t["art_bg"], bd=2, relief=tk.SUNKEN)
        art_frame.pack(pady=5)
        self.album_image_label = tk.Label(art_frame, text="No Art", bg=t["art_bg"], fg="#444")
        self.album_image_label.pack(padx=5, pady=5)

        self.ui_elements["meta_title"] = tk.Label(left_panel, text="", bg=t["art_bg"], fg=fg, font=font_meta)
        self.ui_elements["meta_title"].pack(pady=1)
        self.ui_elements["meta_artist"] = tk.Label(left_panel, text="", bg=t["art_bg"], fg=t["accent"], font=font_meta)
        self.ui_elements["meta_artist"].pack(pady=1)
        self.ui_elements["meta_album"] = tk.Label(left_panel, text="", bg=t["art_bg"], fg=fg, font=font_small)
        self.ui_elements["meta_album"].pack(pady=1)

        vis_frame = tk.Frame(left_panel, bg=bg)
        vis_frame.pack(pady=10)
        for i in range(12):
            bar = tk.Frame(vis_frame, bg=t["accent"], width=8, height=5)
            bar.pack(side=tk.LEFT, padx=1)
            self.visualizer_bars.append(bar)

        right_panel = tk.Frame(main_frame, bg=bg)
        right_panel.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)

        progress_frame = tk.Frame(right_panel, bg=bg)
        progress_frame.pack(fill=tk.X, pady=5)
        self.ui_elements["progress_label"] = tk.Label(progress_frame, text="0:00", bg=bg, fg=fg, font=font_small)
        self.ui_elements["progress_label"].pack(side=tk.LEFT)

        tk.Button(progress_frame, text="<<", command=self.skip_backward, bg=btn_bg, fg=fg, font=font_btn, width=3).pack(side=tk.LEFT, padx=2)
        
        self.ui_elements["progress"] = tk.Canvas(progress_frame, height=20, bg=t["highlight"], highlightthickness=0, width=500)
        self.ui_elements["progress"].pack(side=tk.LEFT, fill=tk.X, expand=True, padx=2)
        self.ui_elements["progress"].bind("<Button-1>", self._scrub_audio)
        self.ui_elements["progress"].bind("<ButtonRelease-1>", self._scrub_audio)
        self.ui_elements["progress_current"] = 0
        
        tk.Button(progress_frame, text=">>", command=self.skip_forward, bg=btn_bg, fg=fg, font=font_btn, width=3).pack(side=tk.LEFT, padx=2)

        self.ui_elements["progress_end"] = tk.Label(progress_frame, text="0:00", bg=bg, fg=fg, font=font_small)
        self.ui_elements["progress_end"].pack(side=tk.LEFT)

        frame1 = tk.Frame(right_panel, bg=btn_bg)
        frame1.pack(fill=tk.X, pady=3)
        for text, cmd in [("Pause", self.pause_music), ("Unpause", self.unpause_music), ("Skip", self.skip_song),
                          ("Prev", self.prev_song), ("Change Dir", self.change_directory)]:
            btn = tk.Button(frame1, text=text, command=cmd, bg=btn_bg, fg=fg, font=font_btn)
            btn.pack(side=tk.LEFT, padx=1, pady=1)
            if text == "Pause":
                self.ui_elements["pause_btn"] = btn

        frame2 = tk.Frame(right_panel, bg=btn_bg)
        frame2.pack(fill=tk.X, pady=3)
        for text, cmd in [("Save", self.save_song), ("Clear", self.clear_playlist), ("Shuffle", self.shuffle_playlist), ("Smart", self.smart_shuffle)]:
            tk.Button(frame2, text=text, command=cmd, bg=btn_bg, fg=fg, font=font_btn).pack(side=tk.LEFT, padx=1, pady=1)

        frame3 = tk.Frame(right_panel, bg=btn_bg)
        frame3.pack(fill=tk.X, pady=3)
        for text, cmd, key in [("Repeat: Off", self.toggle_repeat, "repeat_btn"), ("Playlist Only: Off", self.toggle_playlist_only, "playlist_only_btn"),
                               ("Sequential: Off", self.toggle_playlist_sequential, "playlist_seq_btn"), ("Dir Seq: Off", self.toggle_dir_sequential, "dir_seq_btn")]:
            btn = tk.Button(frame3, text=text, command=cmd, bg=btn_bg, fg=btn_fg, font=font_btn)
            btn.pack(side=tk.LEFT, padx=1, pady=1)
            if key:
                self.ui_elements[key] = btn

        frame4 = tk.Frame(right_panel, bg=btn_bg)
        frame4.pack(fill=tk.X, pady=3)
        for text, cmd in [("Save Playlist", self.save_playlist_file), ("Load Playlist", self.load_playlist_file), ("Look Up", self.lookup_song),
                          ("Play Video", self.play_video), ("Lyrics", self.fetch_lyrics), ("Help", self.show_help)]:
            tk.Button(frame4, text=text, command=cmd, bg=btn_bg, fg=btn_fg, font=font_btn).pack(side=tk.LEFT, padx=1, pady=1)

        theme_frame = tk.Frame(right_panel, bg=bg)
        theme_frame.pack(fill=tk.X, pady=3)
        tk.Button(theme_frame, text=f"Theme: {self.state.current_theme}", command=self.cycle_theme, bg=btn_bg, fg=btn_fg, font=font_btn).pack(side=tk.LEFT, padx=1)
        self.ui_elements["eq_btn"] = tk.Button(theme_frame, text=f"EQ: {self.state.eq_preset}", command=self.cycle_eq_preset, bg=btn_bg, fg=btn_fg, font=font_btn)
        self.ui_elements["eq_btn"].pack(side=tk.LEFT, padx=1)
        sleep_frame = tk.Frame(theme_frame, bg=bg)
        sleep_frame.pack(side=tk.LEFT, padx=5)
        self.ui_elements["sleep_entry"] = tk.Entry(sleep_frame, width=4, bg=t["highlight"], fg=fg, font=font_small)
        self.ui_elements["sleep_entry"].insert(0, "0")
        self.ui_elements["sleep_entry"].pack(side=tk.LEFT, padx=2)
        tk.Button(sleep_frame, text="Sleep", command=lambda: self.set_sleep_timer(int(self.ui_elements["sleep_entry"].get() or 0)), bg=btn_bg, fg=btn_fg, font=font_btn).pack(side=tk.LEFT, padx=2)

        playing_frame = tk.Frame(right_panel, bg=t["highlight"], bd=2, relief=tk.GROOVE)
        playing_frame.pack(pady=5, fill=tk.X)
        self.ui_elements["playing"] = tk.Label(playing_frame, text="", bg=t["highlight"], fg=t["playing_fg"], font=font_ui, anchor="w")
        self.ui_elements["playing"].pack(fill=tk.X)

        search_frame = tk.Frame(right_panel, bg=bg)
        search_frame.pack(fill=tk.X, pady=3)
        tk.Label(search_frame, text="Search:", bg=bg, fg=fg, font=font_small).pack(side=tk.LEFT, padx=2)
        self.ui_elements["search_entry"] = tk.Entry(search_frame, bg=t["highlight"], fg=fg, font=font_small)
        self.ui_elements["search_entry"].pack(side=tk.LEFT, fill=tk.X, expand=True, padx=2)
        self.ui_elements["search_entry"].bind("<KeyRelease>", self.on_search)

        list_frame = tk.Frame(right_panel, bg=bg)
        list_frame.pack(pady=5, fill=tk.BOTH, expand=True)
        scroll = tk.Scrollbar(list_frame)
        scroll.pack(side=tk.RIGHT, fill=tk.Y)
        lb = tk.Listbox(list_frame, bg=t["highlight"], fg=fg, font=font_ui, yscrollcommand=scroll.set,
                        selectbackground="#00FF00", selectforeground="#000000")
        lb.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        lb.bind("<<ListboxSelect>>", self.play_selected)
        self.ui_elements["playlist"] = lb
        scroll.config(command=lb.yview)

        vol_frame = tk.Frame(right_panel, bg=bg)
        vol_frame.pack(pady=5)
        self.ui_elements["volume_label"] = tk.Label(vol_frame, text="Volume: 50%", bg=bg, fg=fg, font=font_ui, padx=20)
        self.ui_elements["volume_label"].pack(pady=5)
        vol_btn_frame = tk.Frame(vol_frame, bg=bg)
        vol_btn_frame.pack()
        for text, cmd in [("-", self.decrease_volume), ("+", self.increase_volume)]:
            tk.Button(vol_btn_frame, text=text, command=cmd, bg=btn_bg, fg=btn_fg, font=font_btn).pack(side=tk.LEFT, padx=10)

        self.ui_elements["error"] = tk.Label(self.root, text="", bg=bg, fg="yellow", font=font_ui)
        self.ui_elements["error"].pack(pady=2)
        tk.Button(self.root, text="Exit", command=self.on_close, bg=btn_bg, fg="red", font=font_btn).pack(pady=2)


def main():
    try:
        app = MusicPlayerApp()
        app.run()
    except Exception as e:
        logger.error(f"Startup error: {e}")
        print(f"Error: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
