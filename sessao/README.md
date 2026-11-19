# Sessão KalangoOS

- `rodar.sh`: abre a sessão inteira numa janela do seu desktop atual, num niri aninhado com D-Bus próprio. Inclui
  shell, tela de aprovação e kalangod em simulação. Quando um binário é recompilado, ele é reiniciado sozinho. Dentro
  da janela, a tecla Mod é o Alt: Alt+Espaço abre a barra de comando, Alt+A todos os apps, Alt+Shift+E o menu de
  energia, Alt+T o terminal e Alt+Q fecha a janela.
- `niri.kdl.modelo`: configuração do compositor (cores e raios do Design System, sem sombras, atalhos do KalangoOS).
  `gerar.sh` produz a versão instalada.
- `dist/`: arquivos da imagem do sistema. A imagem precisa dos pacotes `wlsunset` (luz noturna, pelo controle de gama do niri) e `fonts-inter` (a fonte de texto do Design System; títulos usam
  URW Gothic, de `fonts-urw-base35`). Para a sessão de teste, `poc/setup-deps.sh` baixa o Inter sem sudo e
  `rodar.sh` (e portanto `testar.sh`) o usa via `FONTCONFIG_FILE`.

  | Arquivo | Onde vai |
  |---|---|
  | `kalango.desktop` | `/usr/share/wayland-sessions/` |
  | `kalango-sessao` | `/usr/libexec/kalango/` |
  | `kalango-shell.service` (e `crates/kalango-aprovacao/dist/kalango-aprovacao.service`) | `/usr/lib/systemd/user/` |
  | `pam/kalango-bloqueio` (a senha da tela de bloqueio) | `/etc/pam.d/` |
  | `greetd/config.toml` (a tela de entrada no greetd; pacote `greetd` e a conta `greeter`) | `/etc/greetd/` |
  | `kalango-entrada-sessao` (o compositor da tela de entrada) | `/usr/libexec/kalango/` |
  | `entrada.kdl` (niri só com a tela de entrada, que sai quando ela termina) | `/usr/share/kalango/` |
  | `target/release/kalango-entrada` | `/usr/bin/` |
  | `share/applications/os.kalango.Agentes.desktop` (a Central de agentes) | `/usr/share/applications/` |
  | `convidado/kalango-convidado.conf` (a conta do convidado) | `/usr/lib/sysusers.d/` |
  | `convidado/greetd.pam` (o convidado entra sem senha só pela tela de entrada) | `/etc/pam.d/greetd` |
  | `convidado/limpar-convidado` (a pasta do convidado volta vazia ao sair) | `/usr/libexec/kalango/` |
  | `wallpapers/sdrustos-16x9-4k.png`, `lunar-tall.png` e `lunar-wide.png` (o shell põe em cada tela a versão da proporção dela, com `swaybg`; a tela de bloqueio e as sobreposições desenham a mesma) | `/usr/share/kalango/papel-de-parede.png`, `papel-de-parede-alto.png`, `papel-de-parede-largo.png` |

Tela de entrada: é o greeter do greetd. Escolher a pessoa, a senha (o PAM do greetd confere) e a sessão
(`/usr/share/wayland-sessions`); quem já tem uma sessão aberta volta para ela (logind). O convidado aparece quando o
sistema tem a conta `convidado` (`convidado/`: entra sem senha só pela tela de entrada e a pasta pessoal volta vazia
ao sair). Os testes usam um greetd de mentira
(`ferramentas/greetd-teste.py`), com uma senha de teste.

Tela de bloqueio: usa o protocolo de bloqueio do compositor (ext-session-lock-v1), então enquanto ela está
ativa só ela é desenhada e recebe teclado e mouse, e se o shell cair a tela continua bloqueada; o shell que volta
(o supervisor o reinicia) bloqueia de novo para a pessoa poder digitar a senha. A senha é conferida pelo PAM
fora da interface. Fora da sessão de teste ela também atende ao `Lock` e ao `Unlock` do logind
(`loginctl lock-session`). Os testes usam `KALANGO_PAM_DIR` com uma senha de teste, nunca a da pessoa.

Supervisão: o shell e a tela de aprovação são serviços do usuário presos a `graphical-session.target`, com
`Restart=always` e reinício em 200 ms. Medido: depois de um `kill -9`, a barra, a dock e a área voltam em 407–433 ms.
