#!/usr/bin/env python3
"""A stand-in for Open-Meteo (tests only: the shell uses it through KALANGO_CLIMA_URL), and for the agents' web search
(kalangod, through KALANGOD_BUSCA_URL). usage: clima-teste.py <port> [log]
Answers /v1/search (one city) and /v1/forecast (clear sky, 19 °C now, the next hours and today's high and low); /search,
a results page in Bing's markup (the first result's address through Bing's counter, as the real one); /js, a page whose
text exists only after its script runs — and the script also tries another name for this machine (localhost), which
the browser's proxy must refuse. Each request's path goes to the log, so a test can check what was asked."""
import json, sys, datetime
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1])
LOG = sys.argv[2] if len(sys.argv) > 2 else None


class H(BaseHTTPRequestHandler):
    def do_GET(self):
        if LOG:
            with open(LOG, "a") as f:
                f.write(self.path + "\n")
        if self.path.startswith("/search") and any(w in __import__("urllib.parse").parse.unquote_plus(self.path).lower()
                                                   for w in ("resultado", "placar", "ontem", "último", "ultimo", "venceu", "result")):
            # a result question: the team page first, its summary generic (the score is only inside the page)
            port = self.server.server_address[1]
            page = f"""<html><head><title>resultados</title></head><body><ol id="b_results">
<li class="b_algo"><h2><a href="http://127.0.0.1:{port}/time-kalango">Kalango FC | Últimas notícias, resultados e próximos jogos</a></h2>
<div class="b_caption"><p class="b_lineclamp2">Próximos jogos do Kalango FC, resultados, vídeos, reforços, escalação e notícias ao vivo.</p></div></li>
<li class="b_algo"><h2><a href="http://127.0.0.1:{port}/kalango-oficial">Kalango FC - site oficial</a></h2><p>Notícias, ingressos e a loja do clube.</p></li>
</ol></body></html>"""
            return self.answer(page.encode(), "text/html; charset=utf-8")
        if self.path.startswith("/time-kalango"):
            page = """<html><head><title>Kalango FC | Últimas notícias</title></head><body>
<a href="/tabela-kalango" style="position:fixed;left:0;top:0;right:0;bottom:0;display:block" aria-label="Ver a tabela"></a><h1>Kalango FC</h1><ul>
<li>Há 11 horas · Com dois de Tatu, Kalango FC vence o Lagarto por 3 a 1 no Estádio da Serra e sobe na tabela</li>
<li>Há 18 horas · Clube anuncia a venda de ingressos para o próximo jogo</li>
<li>Há 2 dias · Técnico fala sobre a sequência de jogos em casa</li></ul></body></html>"""
            return self.answer(page.encode(), "text/html; charset=utf-8")
        if self.path.startswith("/privado-kalango"):
            # (a page behind a login: 403 to anyone, a browser too)
            data = "<html><head><title>Entrar</title></head><body><h1>Entre para ver os dados do clube</h1></body></html>".encode()
            self.send_response(403)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if self.path.startswith("/tabela-kalango"):
            page = """<html><head><title>Tabela · Kalango FC</title></head><body><h1>Tabela do campeonato</h1>
<p>1º Tatus · 2º Lagarto · 3º Kalango FC</p></body></html>"""
            return self.answer(page.encode(), "text/html; charset=utf-8")
        if self.path.startswith("/search"):
            page = """<html><head><title>resultados</title></head><body><ol id="b_results">
<li class="b_algo"><h2><a href="https://www.bing.com/ck/a?!&amp;&amp;p=1&amp;u=a1aHR0cHM6Ly9leGVtcGxvLmNvbS5ici9qb2dvLWRvLWthbGFuZ28&amp;ntb=1">Jogo do Kalango FC hoje: horário e onde assistir</a></h2>
<div class="b_caption"><p class="b_lineclamp2">2 horas atrás · O Kalango FC enfrenta o Lagarto hoje, às 21h30, no Estádio da Serra.</p></div></li>
<li class="b_algo"><h2><a href="https://exemplo.com.br/agenda">Agenda do Kalango FC</a></h2><p>Calendário com as próximas rodadas.</p></li>
</ol></body></html>"""
            return self.answer(page.encode(), "text/html; charset=utf-8")
        if self.path.startswith("/js"):
            port = self.server.server_address[1]
            page = f"""<html><head><title>montada por script</title></head><body><div id="a"></div>
<script>document.getElementById("a").textContent = "Conteúdo montado por script: o Kalango FC joga às 21h30.";
fetch("http://localhost:{port}/vazou").catch(() => {{}});</script></body></html>"""
            return self.answer(page.encode(), "text/html; charset=utf-8")
        if self.path.startswith("/v1/search"):
            body = {"results": [{"name": "São Paulo", "admin1": "São Paulo", "country": "Brasil", "latitude": -23.55, "longitude": -46.63}]}
        elif self.path.startswith("/v1/forecast"):
            now = datetime.datetime.now().replace(minute=0, second=0, microsecond=0)
            times = [(now + datetime.timedelta(hours=h)).strftime("%Y-%m-%dT%H:00") for h in range(0, 30)]
            temps = [round(19 + 6 * __import__("math").sin((now.hour + h - 9) / 24 * 6.283), 1) for h in range(0, 30)]
            body = {"current": {"time": now.strftime("%Y-%m-%dT%H:%M"), "temperature_2m": 19.2, "weather_code": 0},
                    "hourly": {"time": times, "temperature_2m": temps},
                    "daily": {"temperature_2m_max": [27.0, 28.0], "temperature_2m_min": [16.0, 17.0]}}
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.answer(json.dumps(body).encode(), "application/json")

    def answer(self, data, kind):
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *a):
        pass


HTTPServer(("127.0.0.1", PORT), H).serve_forever()
